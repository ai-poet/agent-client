import { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, ScrollView, Text, TextInput, View } from "react-native";
import {
  Box,
  Check,
  ChevronDown,
  Download,
  RefreshCcw,
  Search,
  ShieldCheck,
} from "lucide-react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import type { ContextHubMarketplaceSkillEntry } from "@server/shared/messages";
import { MenuHeader } from "@/components/headers/menu-header";
import { isWeb } from "@/constants/platform";
import { useToast } from "@/contexts/toast-context";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useHostRuntimeClient } from "@/runtime/host-runtime";
import { useNavigationActiveWorkspaceSelection } from "@/stores/navigation-active-workspace-store";
import { useSessionStore } from "@/stores/session-store";
import { toErrorMessage } from "@/utils/error-messages";

function normalizeSearch(value: string): string {
  return value.trim();
}

function permissionLabels(skill: ContextHubMarketplaceSkillEntry): string[] {
  const labels: string[] = [];
  if (skill.permissions.network) labels.push("network");
  if (skill.permissions.filesystem) labels.push("filesystem");
  if (skill.permissions.subprocess) labels.push("subprocess");
  if (skill.permissions.envVars.length > 0) {
    labels.push(`env ${skill.permissions.envVars.length}`);
  }
  return labels.length > 0 ? labels : ["no elevated permissions"];
}

function skillStats(skill: ContextHubMarketplaceSkillEntry): string {
  const age =
    skill.daysSinceUpdate === null ? "updated recently" : `${skill.daysSinceUpdate}d since update`;
  return `${skill.trustLevel} · ${skill.downloadCount} downloads · ${skill.downloads7d}/7d · ${age}`;
}

interface SkillLibraryScreenProps {
  serverId: string;
}

export function SkillLibraryScreen({ serverId }: SkillLibraryScreenProps) {
  const { theme } = useUnistyles();
  const toast = useToast();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebar, [locale]);
  const activeWorkspaceSelection = useNavigationActiveWorkspaceSelection();
  const activeWorkspaceId =
    activeWorkspaceSelection?.serverId === serverId ? activeWorkspaceSelection.workspaceId : "";
  const client = useHostRuntimeClient(serverId);
  const workspaces = useSessionStore((state) =>
    Array.from(state.sessions[serverId]?.workspaces.values() ?? []),
  );
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState(activeWorkspaceId);
  const [isWorkspacePickerOpen, setIsWorkspacePickerOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [skills, setSkills] = useState<ContextHubMarketplaceSkillEntry[]>([]);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [installingSkillId, setInstallingSkillId] = useState<string | null>(null);

  useEffect(() => {
    if (activeWorkspaceId) {
      setSelectedWorkspaceId(activeWorkspaceId);
    }
  }, [activeWorkspaceId]);

  useEffect(() => {
    if (!selectedWorkspaceId && workspaces[0]) {
      setSelectedWorkspaceId(workspaces[0].id);
    }
  }, [selectedWorkspaceId, workspaces]);

  const workspace = useMemo(
    () => workspaces.find((entry) => entry.id === selectedWorkspaceId) ?? null,
    [selectedWorkspaceId, workspaces],
  );

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(normalizeSearch(searchQuery)), 250);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  const refresh = useCallback(async () => {
    if (!client) {
      setSkills([]);
      setErrorMessage("Host is not connected.");
      return;
    }
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const result = await client.skillsMarketplaceList({
        query: debouncedQuery || undefined,
        limit: 30,
        minTrust: "verified",
        workspaceId: workspace?.id,
        cwd: workspace?.workspaceDirectory,
      });
      setSkills(result.skills);
      setErrorMessage(result.error);
    } catch (error) {
      setErrorMessage(toErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }, [client, debouncedQuery, workspace?.id, workspace?.workspaceDirectory]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const installSkill = useCallback(
    async (skill: ContextHubMarketplaceSkillEntry, overwrite = false) => {
      if (!client || !workspace) {
        toast.error("Open a workspace before installing skills.");
        return;
      }
      setInstallingSkillId(skill.id);
      try {
        const result = await client.skillsMarketplaceInstall({
          workspaceId: workspace.id,
          cwd: workspace.workspaceDirectory,
          skillId: skill.id,
          name: skill.name,
          version: skill.version ?? undefined,
          overwrite,
        });
        if (result.error) {
          if (result.conflict && !overwrite) {
            toast.error(`${skill.name} already exists with different content.`);
          } else {
            toast.error(result.error);
          }
          return;
        }
        toast.show(`${skill.name} installed. Reload running agents to pick it up.`, {
          variant: "success",
        });
        await refresh();
      } catch (error) {
        toast.error(toErrorMessage(error));
      } finally {
        setInstallingSkillId(null);
      }
    },
    [client, refresh, toast, workspace],
  );

  const headerActions = (
    <View style={styles.headerActions}>
      <Pressable
        style={styles.ghostAction}
        accessibilityRole="button"
        onPress={() => void refresh()}
        disabled={isLoading}
      >
        <RefreshCcw size={16} color={theme.colors.foregroundMuted} />
        <Text style={styles.ghostActionText}>
          {isLoading ? (locale === "zh" ? "正在刷新" : "Refreshing") : text.skillRefresh}
        </Text>
      </Pressable>
      <View style={styles.searchBox}>
        <Search size={16} color={theme.colors.foregroundMuted} />
        <TextInput
          value={searchQuery}
          onChangeText={setSearchQuery}
          placeholder={text.skillSearchPlaceholder}
          placeholderTextColor={theme.colors.foregroundMuted}
          style={[styles.searchInput, isWeb && ({ outlineStyle: "none" } as any)]}
          testID="skill-library-search"
        />
      </View>
    </View>
  );

  const workspaceLabel = workspace
    ? `${workspace.projectDisplayName || workspace.name} · ${workspace.name}`
    : locale === "zh"
      ? "选择一个工作区后即可安装技能"
      : "Choose a workspace to install skills";

  return (
    <View style={styles.container}>
      <MenuHeader rightContent={headerActions} borderless />
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator
        testID="skill-library-page"
      >
        <View style={styles.hero}>
          <Text style={styles.title}>{text.skills}</Text>
          <Text style={styles.subtitle}>AI Skill Store · CodexCLI · verified</Text>
          <View style={styles.workspacePickerWrap}>
            <Pressable
              style={styles.workspacePicker}
              accessibilityRole="button"
              testID="skill-library-workspace-picker"
              onPress={() => setIsWorkspacePickerOpen((value) => !value)}
            >
              <Text style={styles.workspaceHint} numberOfLines={1}>
                {workspaceLabel}
              </Text>
              <ChevronDown size={16} color={theme.colors.foregroundMuted} />
            </Pressable>
            {isWorkspacePickerOpen ? (
              <View style={styles.workspacePickerMenu}>
                {workspaces.map((entry) => (
                  <Pressable
                    key={entry.id}
                    style={styles.workspacePickerItem}
                    accessibilityRole="button"
                    testID={`skill-library-workspace-option-${entry.id}`}
                    onPress={() => {
                      setSelectedWorkspaceId(entry.id);
                      setIsWorkspacePickerOpen(false);
                    }}
                  >
                    <Text style={styles.workspacePickerItemText} numberOfLines={1}>
                      {entry.projectDisplayName || entry.name} · {entry.name}
                    </Text>
                    {entry.id === workspace?.id ? (
                      <Check size={14} color={theme.colors.accentBright} />
                    ) : null}
                  </Pressable>
                ))}
                {workspaces.length === 0 ? (
                  <Text style={styles.workspacePickerEmpty}>
                    {locale === "zh" ? "暂无工作区" : "No workspaces"}
                  </Text>
                ) : null}
              </View>
            ) : null}
          </View>
        </View>

        {errorMessage ? (
          <View style={styles.notice}>
            <Text style={styles.noticeText}>{errorMessage}</Text>
          </View>
        ) : null}

        <View style={styles.sectionHeader}>
          <Text style={styles.sectionTitle}>
            {locale === "zh" ? "市场技能" : "Marketplace skills"}
          </Text>
          <Text style={styles.sectionCount}>{skills.length}</Text>
        </View>

        <View style={styles.skillGrid}>
          {skills.map((skill) => {
            const isInstalling = installingSkillId === skill.id;
            const canInstall = Boolean(client && workspace) && !isInstalling;
            return (
              <View key={skill.id} style={styles.skillRow} testID={`skill-marketplace-row-${skill.id}`}>
                <View style={styles.skillIcon}>
                  <Box size={22} color={theme.colors.accentBright} />
                </View>
                <View style={styles.skillCopy}>
                  <View style={styles.skillTitleRow}>
                    <Text style={styles.skillName} numberOfLines={1}>
                      {skill.name}
                    </Text>
                    {skill.installed ? (
                      <View style={styles.installedBadge}>
                        <Check size={12} color={theme.colors.success} />
                        <Text style={styles.installedText}>{text.skillInstalled}</Text>
                      </View>
                    ) : null}
                  </View>
                  <Text style={styles.skillDescription} numberOfLines={2}>
                    {skill.description}
                  </Text>
                  <View style={styles.pills}>
                    <View style={styles.pill}>
                      <ShieldCheck size={12} color={theme.colors.foregroundMuted} />
                      <Text style={styles.pillText}>{skill.trustLevel}</Text>
                    </View>
                    {permissionLabels(skill).map((label) => (
                      <View key={label} style={styles.pill}>
                        <Text style={styles.pillText}>{label}</Text>
                      </View>
                    ))}
                  </View>
                  <Text style={styles.skillSource} numberOfLines={1}>
                    {skillStats(skill)}
                  </Text>
                </View>
                <Pressable
                  style={[styles.installAction, !canInstall ? styles.installActionDisabled : null]}
                  accessibilityRole="button"
                  testID={`skill-marketplace-install-${skill.id}`}
                  onPress={() => void installSkill(skill)}
                  disabled={!canInstall}
                >
                  {skill.installed ? (
                    <Check size={18} color={theme.colors.foregroundMuted} />
                  ) : (
                    <Download
                      size={18}
                      color={canInstall ? theme.colors.palette.black : theme.colors.foregroundMuted}
                    />
                  )}
                  <Text
                    style={[styles.installActionText, !canInstall ? styles.disabledText : null]}
                  >
                    {isInstalling
                      ? locale === "zh"
                        ? "安装中"
                        : "Installing"
                      : skill.installed
                        ? text.skillInstalled
                        : locale === "zh"
                          ? "安装"
                          : "Install"}
                  </Text>
                </Pressable>
              </View>
            );
          })}
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  scroll: {
    flex: 1,
  },
  content: {
    width: "100%",
    maxWidth: 1480,
    alignSelf: "center",
    paddingHorizontal: theme.spacing[8],
    paddingBottom: theme.spacing[16],
    gap: theme.spacing[8],
  },
  headerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
  },
  ghostAction: {
    minHeight: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
  },
  ghostActionText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  searchBox: {
    minWidth: 260,
    height: 40,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    padding: 0,
  },
  hero: {
    paddingTop: theme.spacing[16],
    gap: theme.spacing[2],
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize["4xl"],
    fontWeight: theme.fontWeight.normal,
  },
  subtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.lg,
  },
  workspaceHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    flex: 1,
    minWidth: 0,
  },
  workspacePickerWrap: {
    position: "relative",
    maxWidth: 420,
  },
  workspacePicker: {
    minHeight: 34,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  workspacePickerMenu: {
    position: "absolute",
    top: 40,
    left: 0,
    right: 0,
    zIndex: 10,
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface2,
    paddingVertical: theme.spacing[1],
  },
  workspacePickerItem: {
    minHeight: 36,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
  },
  workspacePickerItemText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    flex: 1,
    minWidth: 0,
  },
  workspacePickerEmpty: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  notice: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    borderRadius: theme.borderRadius.md,
    padding: theme.spacing[4],
  },
  noticeText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
  },
  sectionTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
  },
  sectionCount: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  skillGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    columnGap: theme.spacing[10],
    rowGap: theme.spacing[8],
  },
  skillRow: {
    width: "47%",
    minWidth: 380,
    minHeight: 132,
    flexDirection: "row",
    alignItems: "flex-start",
    gap: theme.spacing[4],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.borderAccent,
    paddingBottom: theme.spacing[5],
  },
  skillIcon: {
    width: 52,
    height: 52,
    borderRadius: theme.borderRadius.md,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.surface1,
    flexShrink: 0,
  },
  skillCopy: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[2],
  },
  skillTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    minWidth: 0,
  },
  skillName: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
    flexShrink: 1,
    minWidth: 0,
  },
  installedBadge: {
    minHeight: 22,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.sm,
    backgroundColor: theme.colors.surface1,
  },
  installedText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  skillDescription: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 19,
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  pill: {
    minHeight: 24,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.sm,
    backgroundColor: theme.colors.surface1,
  },
  pillText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  skillSource: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  installAction: {
    minWidth: 96,
    height: 36,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.foreground,
  },
  installActionDisabled: {
    backgroundColor: theme.colors.surface1,
  },
  installActionText: {
    color: theme.colors.palette.black,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
  },
  disabledText: {
    color: theme.colors.foregroundMuted,
  },
}));
