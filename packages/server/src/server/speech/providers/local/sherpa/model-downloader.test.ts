import { EventEmitter } from "node:events";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import pino from "pino";
import { beforeEach, describe, expect, test, vi } from "vitest";

const spawnMock = vi.hoisted(() => vi.fn());

vi.mock("node:child_process", () => ({
  spawn: spawnMock,
}));

import { ensureSherpaOnnxModel, getSherpaOnnxModelDir } from "./model-downloader.js";

function makeTmpDir(): string {
  return mkdtempSync(path.join(tmpdir(), "paseo-speech-models-"));
}

const logger = pino({ level: "silent" });

type ChildProcessStub = EventEmitter & {
  stdout: EventEmitter;
  stderr: EventEmitter;
};

function makeChildProcessStub(): ChildProcessStub {
  const child = new EventEmitter() as ChildProcessStub;
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  return child;
}

function writeKittenModelFiles(modelsDir: string): void {
  const modelDir = getSherpaOnnxModelDir(modelsDir, "kitten-nano-en-v0_1-fp16");
  mkdirSync(path.join(modelDir, "espeak-ng-data"), { recursive: true });
  writeFileSync(path.join(modelDir, "model.fp16.onnx"), "x");
  writeFileSync(path.join(modelDir, "voices.bin"), "x");
  writeFileSync(path.join(modelDir, "tokens.txt"), "x");
}

describe("sherpa model downloader", () => {
  beforeEach(() => {
    spawnMock.mockReset();
  });

  test("getSherpaOnnxModelDir maps modelId to extractedDir", () => {
    const modelsDir = "/tmp/models";
    expect(getSherpaOnnxModelDir(modelsDir, "parakeet-tdt-0.6b-v3-int8")).toContain(
      "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
    );
    expect(getSherpaOnnxModelDir(modelsDir, "pocket-tts-onnx-int8")).toContain(
      "pocket-tts-onnx-int8",
    );
  });

  test("ensureSherpaOnnxModel succeeds without downloading when files exist", async () => {
    const modelsDir = makeTmpDir();
    const modelDir = getSherpaOnnxModelDir(modelsDir, "kitten-nano-en-v0_1-fp16");

    mkdirSync(path.join(modelDir, "espeak-ng-data"), { recursive: true });
    writeFileSync(path.join(modelDir, "model.fp16.onnx"), "x");
    writeFileSync(path.join(modelDir, "voices.bin"), "x");
    writeFileSync(path.join(modelDir, "tokens.txt"), "x");

    const out = await ensureSherpaOnnxModel({
      modelsDir,
      modelId: "kitten-nano-en-v0_1-fp16",
      logger,
    });

    expect(out).toBe(modelDir);
  });

  test("ensureSherpaOnnxModel logs lifecycle events without progress spam", async () => {
    const modelsDir = makeTmpDir();
    const infoMessages: string[] = [];

    const loggerWithSpy = {
      child: () => loggerWithSpy,
      info: (_obj?: unknown, msg?: string) => {
        if (typeof msg === "string") {
          infoMessages.push(msg);
        }
      },
      error: () => undefined,
    } as unknown as pino.Logger;

    const originalFetch = globalThis.fetch;
    const payload = Buffer.alloc(128 * 1024, 7);
    const fetchMock = vi.fn(async () => {
      return new Response(payload, {
        status: 200,
        headers: { "content-length": String(payload.length) },
      });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    try {
      await ensureSherpaOnnxModel({
        modelsDir,
        modelId: "pocket-tts-onnx-int8",
        logger: loggerWithSpy,
      });
    } finally {
      globalThis.fetch = originalFetch;
    }

    expect(fetchMock).toHaveBeenCalled();
    expect(infoMessages).toContain("Starting model download");
    expect(infoMessages).toContain("Model download completed");
    expect(infoMessages).not.toContain("Downloading model artifact");
  });

  test("extracts tar archives with hidden piped child process output", async () => {
    const modelsDir = makeTmpDir();
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(async () => {
      return new Response(Buffer.from("archive"), { status: 200 });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    spawnMock.mockImplementation((_command: string, args: string[]) => {
      const child = makeChildProcessStub();
      queueMicrotask(() => {
        const destFlagIndex = args.indexOf("-C");
        const destDir = destFlagIndex >= 0 ? args[destFlagIndex + 1] : null;
        if (destDir) {
          writeKittenModelFiles(destDir);
        }
        child.emit("close", 0, null);
      });
      return child;
    });

    try {
      await ensureSherpaOnnxModel({
        modelsDir,
        modelId: "kitten-nano-en-v0_1-fp16",
        logger,
      });
    } finally {
      globalThis.fetch = originalFetch;
    }

    expect(fetchMock).toHaveBeenCalled();
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(spawnMock).toHaveBeenCalledWith(
      "tar",
      [
        "xf",
        path.join(modelsDir, ".downloads", "kitten-nano-en-v0_1-fp16.tar.bz2"),
        "-C",
        modelsDir,
      ],
      {
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      },
    );
  });

  test("includes tar output tail when archive extraction fails", async () => {
    const modelsDir = makeTmpDir();
    const originalFetch = globalThis.fetch;
    globalThis.fetch = vi.fn(async () => {
      return new Response(Buffer.from("archive"), { status: 200 });
    }) as typeof fetch;

    spawnMock.mockImplementation(() => {
      const child = makeChildProcessStub();
      queueMicrotask(() => {
        child.stdout.emit("data", "extracting model\n");
        child.stderr.emit("data", "tar: bad archive\n");
        child.emit("close", 2, null);
      });
      return child;
    });

    let caught: unknown;
    try {
      await ensureSherpaOnnxModel({
        modelsDir,
        modelId: "kitten-nano-en-v0_1-fp16",
        logger,
      });
    } catch (error) {
      caught = error;
    } finally {
      globalThis.fetch = originalFetch;
    }

    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toContain("tar exited with code 2");
    expect((caught as Error).message).toContain("stdout:\nextracting model");
    expect((caught as Error).message).toContain("stderr:\ntar: bad archive");
  });
});
