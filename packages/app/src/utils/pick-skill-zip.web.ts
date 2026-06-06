import { blobToBase64 } from "@/attachments/utils";

export async function pickSkillZipBase64(): Promise<{ name: string; base64: string } | null> {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".zip,application/zip,application/x-zip-compressed";
  const file = await new Promise<File | null>((resolve) => {
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.click();
  });
  if (!file) {
    return null;
  }
  return { name: file.name.replace(/\.zip$/i, ""), base64: await blobToBase64(file) };
}
