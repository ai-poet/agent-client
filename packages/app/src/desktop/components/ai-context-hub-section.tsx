import { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, Text, TextInput, View } from "react-native";
import { StyleSheet } from "react-native-unistyles";
import { Database, FileText, Plug, Send, Wrench } from "lucide-react-native";
import { SettingsSection } from "@/screens/settings/settings-section";
import { Button } from "@/components/ui/button";
import {
  SegmentedControl,
  type SegmentedControlOption,
} from "@/components/ui/segmented-control";
import { Fonts } from "@/constants/theme";
import { useLocalDaemonServerId } from "@/hooks/use-is-local-daemon";
import { useSessionStore } from "@/stores/session-store";
import type {
  ContextHubManagedSkillEntry,
  ContextHubMcpServerProfile,
  ContextHubProjectMemoryItem,
  ContextHubPromptTemplate,
} from "@server/client/daemon-client";

type HubTab = "memory" | "skills" | "prompts" | "mcp";
type McpKind = "stdio" | "http" | "sse";

const TAB_OPTIONS: SegmentedControlOption<HubTab>[] = [
  {
    value: "memory",
    label: "Memory",
    icon: ({ color, size }) => <Database color={color} size={size} />,
  },
  {
    value: "skills",
    label: "Skills",
    icon: ({ color, size }) => <Wrench color={color} size={size} />,
  },
  {
    value: "prompts",
    label: "Prompts",
    icon: ({ color, size }) => <FileText color={color} size={size} />,
  },
  {
    value: "mcp",
    label: "MCP",
    icon: ({ color, size }) => <Plug color={color} size={size} />,
  },
];

function splitTags(value: string): string[] {
  return value
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

function Field({
  label,
  value,
  onChangeText,
  placeholder,
  multiline,
}: {
  label: string;
  value: string;
  onChangeText: (value: string) => void;
  placeholder?: string;
  multiline?: boolean;
}) {
  return (
    <View style={styles.field}>
      <Text style={styles.label}>{label}</Text>
      <TextInput
        value={value}
        onChangeText={onChangeText}
        placeholder={placeholder}
        placeholderTextColor="#7b8190"
        multiline={multiline}
        style={[styles.input, multiline ? styles.textarea : null]}
      />
    </View>
  );
}

export function AIContextHubSection() {
  const localServerId = useLocalDaemonServerId();
  const sessions = useSessionStore((state) => state.sessions);
  const serverId = useMemo(() => {
    if (localServerId && sessions[localServerId]?.client) {
      return localServerId;
    }
    return Object.keys(sessions).find((id) => sessions[id]?.client) ?? null;
  }, [localServerId, sessions]);
  const session = serverId ? sessions[serverId] : null;
  const client = session?.client ?? null;
  const workspaces = useMemo(() => Array.from(session?.workspaces.values() ?? []), [session]);

  const [tab, setTab] = useState<HubTab>("memory");
  const [workspaceId, setWorkspaceId] = useState("");
  const selectedWorkspace = workspaces.find((workspace) => workspace.id === workspaceId) ?? null;
  const focusedAgentId = session?.focusedAgentId ?? null;

  const [status, setStatus] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryItems, setMemoryItems] = useState<ContextHubProjectMemoryItem[]>([]);
  const [memoryTitle, setMemoryTitle] = useState("");
  const [memorySummary, setMemorySummary] = useState("");
  const [memoryDetail, setMemoryDetail] = useState("");
  const [memoryTags, setMemoryTags] = useState("");

  const [skills, setSkills] = useState<ContextHubManagedSkillEntry[]>([]);
  const [skillName, setSkillName] = useState("");
  const [skillDescription, setSkillDescription] = useState("");
  const [skillContent, setSkillContent] = useState("");
  const [skillExport, setSkillExport] = useState("");

  const [prompts, setPrompts] = useState<ContextHubPromptTemplate[]>([]);
  const [promptName, setPromptName] = useState("");
  const [promptContent, setPromptContent] = useState("");
  const [promptArgs, setPromptArgs] = useState("");
  const [renderedPrompt, setRenderedPrompt] = useState("");

  const [mcpProfiles, setMcpProfiles] = useState<ContextHubMcpServerProfile[]>([]);
  const [mcpName, setMcpName] = useState("");
  const [mcpKind, setMcpKind] = useState<McpKind>("stdio");
  const [mcpCommandOrUrl, setMcpCommandOrUrl] = useState("");
  const [mcpArgs, setMcpArgs] = useState("");

  useEffect(() => {
    if (!workspaceId && workspaces[0]) {
      setWorkspaceId(workspaces[0].id);
    }
  }, [workspaceId, workspaces]);

  const refresh = useCallback(async () => {
    if (!client) {
      setStatus("Local daemon is not connected.");
      return;
    }
    setIsBusy(true);
    setStatus(null);
    try {
      const [memory, skillList, promptList, mcpList] = await Promise.all([
        workspaceId
          ? client.memoryList({
              workspaceId,
              query: memoryQuery || undefined,
              pageSize: 50,
            })
          : Promise.resolve({ items: [] as ContextHubProjectMemoryItem[] }),
        client.skillsList({
          workspaceId: workspaceId || undefined,
          cwd: selectedWorkspace?.workspaceDirectory,
        }),
        client.promptsList({ workspaceId: workspaceId || undefined }),
        client.mcpList(),
      ]);
      setMemoryItems(memory.items);
      setSkills(skillList.skills);
      setPrompts(promptList.prompts);
      setMcpProfiles(mcpList.profiles);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  }, [client, memoryQuery, selectedWorkspace?.workspaceDirectory, workspaceId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const createMemory = useCallback(async () => {
    if (!client || !workspaceId) return;
    setIsBusy(true);
    try {
      await client.memoryCreate({
        input: {
          workspaceId,
          title: memoryTitle,
          summary: memorySummary,
          detail: memoryDetail,
          tags: splitTags(memoryTags),
          source: "manual",
        },
      });
      setMemoryTitle("");
      setMemorySummary("");
      setMemoryDetail("");
      setMemoryTags("");
      setStatus("Memory saved.");
      await refresh();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  }, [client, memoryDetail, memorySummary, memoryTags, memoryTitle, refresh, workspaceId]);

  const deleteMemory = useCallback(
    async (item: ContextHubProjectMemoryItem) => {
      if (!client) return;
      await client.memoryDelete({ workspaceId: item.workspaceId, memoryId: item.id });
      await refresh();
    },
    [client, refresh],
  );

  const importSkill = useCallback(async () => {
    if (!client) return;
    setIsBusy(true);
    try {
      await client.skillsImport({
        name: skillName,
        content: skillContent,
        description: skillDescription || undefined,
      });
      setSkillName("");
      setSkillDescription("");
      setSkillContent("");
      setStatus("Skill imported into Paseo-managed skills.");
      await refresh();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  }, [client, refresh, skillContent, skillDescription, skillName]);

  const exportSkill = useCallback(
    async (skill: ContextHubManagedSkillEntry) => {
      if (!client) return;
      const result = await client.skillsExport({
        skillId: skill.id,
        workspaceId: workspaceId || undefined,
        cwd: selectedWorkspace?.workspaceDirectory,
      });
      setSkillExport(result.content ?? "");
      setTab("skills");
    },
    [client, selectedWorkspace?.workspaceDirectory, workspaceId],
  );

  const createPrompt = useCallback(async () => {
    if (!client) return;
    setIsBusy(true);
    try {
      await client.promptsCreate({
        input: {
          name: promptName,
          content: promptContent,
          scope: workspaceId ? "workspace" : "global",
          workspaceId: workspaceId || undefined,
        },
      });
      setPromptName("");
      setPromptContent("");
      setStatus("Prompt saved.");
      await refresh();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  }, [client, promptContent, promptName, refresh, workspaceId]);

  const renderPrompt = useCallback(
    async (prompt: ContextHubPromptTemplate, sendToAgent: boolean) => {
      if (!client) return;
      const result = await client.promptsRender({
        promptId: prompt.id,
        argumentsText: promptArgs,
        recordUsage: true,
      });
      setRenderedPrompt(result.text ?? "");
      if (sendToAgent && focusedAgentId && result.text) {
        await client.sendAgentMessage(focusedAgentId, result.text);
        setStatus("Prompt sent to the focused agent.");
      }
    },
    [client, focusedAgentId, promptArgs],
  );

  const deletePrompt = useCallback(
    async (prompt: ContextHubPromptTemplate) => {
      if (!client || prompt.readOnly) return;
      await client.promptsDelete({ promptId: prompt.id });
      await refresh();
    },
    [client, refresh],
  );

  const upsertMcp = useCallback(async () => {
    if (!client) return;
    const server =
      mcpKind === "stdio"
        ? {
            type: "stdio" as const,
            command: mcpCommandOrUrl,
            args: mcpArgs.split(/\s+/).map((arg) => arg.trim()).filter(Boolean),
          }
        : {
            type: mcpKind,
            url: mcpCommandOrUrl,
          };
    setIsBusy(true);
    try {
      await client.mcpUpsert({
        profile: {
          name: mcpName,
          enabled: true,
          workspaceIds: workspaceId ? [workspaceId] : [],
          server,
        },
      });
      setMcpName("");
      setMcpCommandOrUrl("");
      setMcpArgs("");
      setStatus("MCP server saved.");
      await refresh();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    } finally {
      setIsBusy(false);
    }
  }, [client, mcpArgs, mcpCommandOrUrl, mcpKind, mcpName, refresh, workspaceId]);

  const testMcp = useCallback(async () => {
    if (!client) return;
    const server =
      mcpKind === "stdio"
        ? {
            type: "stdio" as const,
            command: mcpCommandOrUrl,
            args: mcpArgs.split(/\s+/).map((arg) => arg.trim()).filter(Boolean),
          }
        : {
            type: mcpKind,
            url: mcpCommandOrUrl,
          };
    const result = await client.mcpTest({
      profile: {
        name: mcpName || "Untitled MCP",
        server,
      },
    });
    setStatus(result.message);
  }, [client, mcpArgs, mcpCommandOrUrl, mcpKind, mcpName]);

  const deleteMcp = useCallback(
    async (profile: ContextHubMcpServerProfile) => {
      if (!client) return;
      await client.mcpDelete({ profileId: profile.id });
      await refresh();
    },
    [client, refresh],
  );

  return (
    <View style={styles.container}>
      <SettingsSection
        title="AI Context Hub"
        trailing={
          <Button size="sm" variant="outline" disabled={isBusy} onPress={() => void refresh()}>
            Refresh
          </Button>
        }
      >
        <View style={styles.workspaceRow}>
          <Text style={styles.label}>Workspace</Text>
          <View style={styles.workspaceList}>
            {workspaces.slice(0, 6).map((workspace) => (
              <Pressable
                key={workspace.id}
                onPress={() => setWorkspaceId(workspace.id)}
                style={[styles.workspaceChip, workspace.id === workspaceId && styles.workspaceChipActive]}
              >
                <Text
                  numberOfLines={1}
                  style={[styles.workspaceChipText, workspace.id === workspaceId && styles.workspaceChipTextActive]}
                >
                  {workspace.name}
                </Text>
              </Pressable>
            ))}
          </View>
        </View>
        <SegmentedControl options={TAB_OPTIONS} value={tab} onValueChange={setTab} />
        {status ? <Text style={styles.status}>{status}</Text> : null}
      </SettingsSection>

      {tab === "memory" ? (
        <SettingsSection title="Project Memory">
          <View style={styles.grid}>
            <View style={styles.panel}>
              <Field label="Search" value={memoryQuery} onChangeText={setMemoryQuery} />
              <Field label="Title" value={memoryTitle} onChangeText={setMemoryTitle} />
              <Field label="Summary" value={memorySummary} onChangeText={setMemorySummary} multiline />
              <Field label="Detail" value={memoryDetail} onChangeText={setMemoryDetail} multiline />
              <Field label="Tags" value={memoryTags} onChangeText={setMemoryTags} placeholder="comma,separated" />
              <Button disabled={!workspaceId || !memorySummary.trim()} onPress={() => void createMemory()}>
                Save Memory
              </Button>
            </View>
            <View style={styles.panel}>
              {memoryItems.map((item) => (
                <View key={item.id} style={styles.item}>
                  <Text style={styles.itemTitle}>{item.title}</Text>
                  <Text style={styles.itemMeta}>
                    {item.kind} · {item.importance}
                  </Text>
                  <Text style={styles.itemBody}>{item.summary}</Text>
                  <Button size="sm" variant="ghost" onPress={() => void deleteMemory(item)}>
                    Delete
                  </Button>
                </View>
              ))}
            </View>
          </View>
        </SettingsSection>
      ) : null}

      {tab === "skills" ? (
        <SettingsSection title="Skills">
          <View style={styles.grid}>
            <View style={styles.panel}>
              <Field label="Name" value={skillName} onChangeText={setSkillName} />
              <Field label="Description" value={skillDescription} onChangeText={setSkillDescription} />
              <Field label="SKILL.md" value={skillContent} onChangeText={setSkillContent} multiline />
              <Button disabled={!skillName.trim() || !skillContent.trim()} onPress={() => void importSkill()}>
                Import Skill
              </Button>
              {skillExport ? <Text style={styles.codeBlock}>{skillExport}</Text> : null}
            </View>
            <View style={styles.panel}>
              {skills.map((skill) => (
                <View key={skill.id} style={styles.item}>
                  <Text style={styles.itemTitle}>{skill.name}</Text>
                  <Text style={styles.itemMeta}>
                    {skill.source} · {skill.readOnly ? "read-only" : "managed"}
                  </Text>
                  {skill.description ? <Text style={styles.itemBody}>{skill.description}</Text> : null}
                  <Button size="sm" variant="ghost" onPress={() => void exportSkill(skill)}>
                    Export
                  </Button>
                </View>
              ))}
            </View>
          </View>
        </SettingsSection>
      ) : null}

      {tab === "prompts" ? (
        <SettingsSection title="Prompt Library">
          <View style={styles.grid}>
            <View style={styles.panel}>
              <Field label="Name" value={promptName} onChangeText={setPromptName} />
              <Field label="Template" value={promptContent} onChangeText={setPromptContent} multiline />
              <Field label="Arguments" value={promptArgs} onChangeText={setPromptArgs} />
              <Button disabled={!promptName.trim() || !promptContent.trim()} onPress={() => void createPrompt()}>
                Save Prompt
              </Button>
              {renderedPrompt ? <Text style={styles.codeBlock}>{renderedPrompt}</Text> : null}
            </View>
            <View style={styles.panel}>
              {prompts.map((prompt) => (
                <View key={prompt.id} style={styles.item}>
                  <Text style={styles.itemTitle}>{prompt.name}</Text>
                  <Text style={styles.itemMeta}>
                    {prompt.source} · used {prompt.usageCount}
                  </Text>
                  <View style={styles.buttonRow}>
                    <Button size="sm" variant="ghost" onPress={() => void renderPrompt(prompt, false)}>
                      Render
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={!focusedAgentId}
                      leftIcon={Send}
                      onPress={() => void renderPrompt(prompt, true)}
                    >
                      Send
                    </Button>
                    {!prompt.readOnly ? (
                      <Button size="sm" variant="ghost" onPress={() => void deletePrompt(prompt)}>
                        Delete
                      </Button>
                    ) : null}
                  </View>
                </View>
              ))}
            </View>
          </View>
        </SettingsSection>
      ) : null}

      {tab === "mcp" ? (
        <SettingsSection title="MCP Servers">
          <View style={styles.grid}>
            <View style={styles.panel}>
              <Field label="Name" value={mcpName} onChangeText={setMcpName} />
              <SegmentedControl
                size="sm"
                value={mcpKind}
                onValueChange={setMcpKind}
                options={[
                  { value: "stdio", label: "stdio" },
                  { value: "http", label: "http" },
                  { value: "sse", label: "sse" },
                ]}
              />
              <Field
                label={mcpKind === "stdio" ? "Command" : "URL"}
                value={mcpCommandOrUrl}
                onChangeText={setMcpCommandOrUrl}
              />
              {mcpKind === "stdio" ? (
                <Field label="Args" value={mcpArgs} onChangeText={setMcpArgs} />
              ) : null}
              <View style={styles.buttonRow}>
                <Button disabled={!mcpName.trim() || !mcpCommandOrUrl.trim()} onPress={() => void upsertMcp()}>
                  Save
                </Button>
                <Button variant="outline" disabled={!mcpCommandOrUrl.trim()} onPress={() => void testMcp()}>
                  Test
                </Button>
              </View>
            </View>
            <View style={styles.panel}>
              {mcpProfiles.map((profile) => (
                <View key={profile.id} style={styles.item}>
                  <Text style={styles.itemTitle}>{profile.name}</Text>
                  <Text style={styles.itemMeta}>
                    {profile.server.type} · {profile.enabled ? "enabled" : "disabled"}
                  </Text>
                  <Text style={styles.itemBody}>
                    {profile.server.type === "stdio" ? profile.server.command : profile.server.url}
                  </Text>
                  <Button size="sm" variant="ghost" onPress={() => void deleteMcp(profile)}>
                    Delete
                  </Button>
                </View>
              ))}
            </View>
          </View>
        </SettingsSection>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    gap: theme.spacing[2],
  },
  workspaceRow: {
    gap: theme.spacing[2],
  },
  workspaceList: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  workspaceChip: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    maxWidth: 180,
  },
  workspaceChipActive: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.surface2,
  },
  workspaceChipText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  workspaceChipTextActive: {
    color: theme.colors.foreground,
  },
  status: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[4],
  },
  panel: {
    flexBasis: 360,
    flexGrow: 1,
    minWidth: 280,
    gap: theme.spacing[3],
  },
  field: {
    gap: theme.spacing[1],
  },
  label: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  input: {
    minHeight: 38,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    color: theme.colors.foreground,
    backgroundColor: theme.colors.surface1,
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    fontSize: theme.fontSize.sm,
  },
  textarea: {
    minHeight: 96,
    textAlignVertical: "top",
  },
  item: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    padding: theme.spacing[3],
    gap: theme.spacing[2],
  },
  itemTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
  },
  itemMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  itemBody: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  buttonRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  codeBlock: {
    color: theme.colors.foreground,
    backgroundColor: theme.colors.surface1,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    padding: theme.spacing[3],
    fontFamily: Fonts.mono,
    fontSize: theme.fontSize.xs,
  },
}));
