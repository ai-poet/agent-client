import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import type {
  ContextHubManagedSkillEntry,
  ContextHubMarketplaceSkillEntry,
} from "@server/shared/messages";
import { MenuHeader } from "@/components/headers/menu-header";
import { isWeb } from "@/constants/platform";
import { useToast } from "@/contexts/toast-context";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useHostRuntimeClient } from "@/runtime/host-runtime";
import { useNavigationActiveWorkspaceSelection } from "@/stores/navigation-active-workspace-store";
import { useWorkspaceList } from "@/stores/session-store-hooks";
import { toErrorMessage } from "@/utils/error-messages";

function normalizeSearch(value: string): string {
  return value.trim();
}

const MARKETPLACE_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");

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

type LocalSkillSummary = {
  key: string;
  name: string;
  description: string | null;
  path: string;
  readOnly: boolean;
  sources: string[];
  scope: "global" | "workspace";
};

function localSkillSourceLabel(source: string, locale: string): string {
  const isZh = locale === "zh";
  switch (source) {
    case "bundled":
      return isZh ? "内置" : "Built-in";
    case "managed":
      return isZh ? "Paseo 管理" : "Paseo managed";
    case "global_agents":
      return isZh ? "全局 Agents" : "Global Agents";
    case "global_claude":
      return isZh ? "全局 Claude" : "Global Claude";
    case "global_codex":
      return isZh ? "全局 Codex" : "Global Codex";
    case "project_agents":
      return isZh ? "工作区 Agents" : "Workspace Agents";
    case "project_claude":
      return isZh ? "工作区 Claude" : "Workspace Claude";
    case "project_codex":
      return isZh ? "工作区 Codex" : "Workspace Codex";
    default:
      return source.replace(/_/g, " ");
  }
}

function summarizeLocalSkills(
  skills: ContextHubManagedSkillEntry[],
  query: string,
): LocalSkillSummary[] {
  const normalizedQuery = query.trim().toLowerCase();
  const summaries = new Map<string, LocalSkillSummary>();
  for (const skill of skills) {
    const haystack = [skill.name, skill.description, skill.source, skill.path]
      .filter(Boolean)
      .join("\n")
      .toLowerCase();
    if (normalizedQuery && !haystack.includes(normalizedQuery)) {
      continue;
    }
    const key = skill.name.trim().toLowerCase();
    const existing = summaries.get(key);
    if (!existing) {
      summaries.set(key, {
        key,
        name: skill.name,
        description: skill.description,
        path: skill.path,
        readOnly: skill.readOnly,
        sources: [skill.source],
        scope: skill.scope,
      });
      continue;
    }
    if (!existing.description && skill.description) {
      existing.description = skill.description;
    }
    if (!existing.sources.includes(skill.source)) {
      existing.sources.push(skill.source);
    }
    if (existing.scope !== "workspace" && skill.scope === "workspace") {
      existing.scope = "workspace";
      existing.path = skill.path;
    }
    existing.readOnly = existing.readOnly && skill.readOnly;
  }
  return Array.from(summaries.values()).sort((a, b) => a.name.localeCompare(b.name));
}

function marketplaceSkillInitial(name: string): string | null {
  const initial = name.trim().match(/[A-Za-z]/)?.[0]?.toUpperCase();
  return initial && /^[A-Z]$/.test(initial) ? initial : null;
}

function groupMarketplaceSkills(skills: ContextHubMarketplaceSkillEntry[]): Array<{
  letter: string;
  skills: ContextHubMarketplaceSkillEntry[];
}> {
  const buckets = new Map<string, ContextHubMarketplaceSkillEntry[]>();
  for (const letter of MARKETPLACE_ALPHABET) {
    buckets.set(letter, []);
  }
  for (const skill of skills) {
    const initial = marketplaceSkillInitial(skill.name);
    if (!initial) {
      continue;
    }
    buckets.get(initial)?.push(skill);
  }
  return MARKETPLACE_ALPHABET.map((letter) => ({
    letter,
    skills: buckets.get(letter) ?? [],
  })).filter((group) => group.skills.length > 0);
}

interface SkillLibraryScreenProps {
  serverId: string;
}

export function SkillLibraryScreen({ serverId }: SkillLibraryScreenProps) {
  const { theme } = useUnistyles();
  const toast = useToast();
  const scrollRef = useRef<ScrollView>(null);
  const marketplaceLetterOffsetsRef = useRef(new Map<string, number>());
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebar, [locale]);
  const activeWorkspaceSelection = useNavigationActiveWorkspaceSelection();
  const activeWorkspaceId =
    activeWorkspaceSelection?.serverId === serverId ? activeWorkspaceSelection.workspaceId : "";
  const client = useHostRuntimeClient(serverId);
  const workspaces = useWorkspaceList(serverId);
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState(
    () => activeWorkspaceId || workspaces[0]?.id || "",
  );
  const [isWorkspacePickerOpen, setIsWorkspacePickerOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [localSkills, setLocalSkills] = useState<ContextHubManagedSkillEntry[]>([]);
  const [marketplaceSkills, setMarketplaceSkills] = useState<ContextHubMarketplaceSkillEntry[]>([]);
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
      setLocalSkills([]);
      setMarketplaceSkills([]);
      setErrorMessage("Host is not connected.");
      return;
    }
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const [localResult, marketplaceResult] = await Promise.allSettled([
        client.skillsList({
          workspaceId: workspace?.id,
          cwd: workspace?.workspaceDirectory,
        }),
        client.skillsMarketplaceList({
          query: debouncedQuery || undefined,
          limit: 260,
          minTrust: "verified",
          workspaceId: workspace?.id,
          cwd: workspace?.workspaceDirectory,
        }),
      ]);
      const errors: string[] = [];
      if (localResult.status === "fulfilled") {
        setLocalSkills(localResult.value.skills);
        if (localResult.value.error) errors.push(localResult.value.error);
      } else {
        setLocalSkills([]);
        errors.push(toErrorMessage(localResult.reason));
      }
      if (marketplaceResult.status === "fulfilled") {
        setMarketplaceSkills(marketplaceResult.value.skills);
        if (marketplaceResult.value.error) errors.push(marketplaceResult.value.error);
      } else {
        setMarketplaceSkills([]);
        errors.push(toErrorMessage(marketplaceResult.reason));
      }
      setErrorMessage(errors[0] ?? null);
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

  const localSkillSummaries = useMemo(
    () => summarizeLocalSkills(localSkills, debouncedQuery),
    [debouncedQuery, localSkills],
  );
  const marketplaceGroups = useMemo(
    () => groupMarketplaceSkills(marketplaceSkills),
    [marketplaceSkills],
  );
  const marketplaceLettersWithResults = useMemo(
    () => new Set(marketplaceGroups.map((group) => group.letter)),
    [marketplaceGroups],
  );
  const showMarketplaceAlphabet = !debouncedQuery && marketplaceGroups.length > 0;
  const marketplaceCountLabel = showMarketplaceAlphabet
    ? `${marketplaceSkills.length} · A-Z`
    : String(marketplaceSkills.length);

  const renderMarketplaceSkillRow = useCallback(
    (skill: ContextHubMarketplaceSkillEntry) => {
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
            <Text style={[styles.installActionText, !canInstall ? styles.disabledText : null]}>
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
    },
    [client, installSkill, installingSkillId, locale, text.skillInstalled, theme, workspace],
  );

  const scrollToMarketplaceLetter = useCallback((letter: string) => {
    const y = marketplaceLetterOffsetsRef.current.get(letter);
    if (typeof y !== "number") {
      return;
    }
    scrollRef.current?.scrollTo({ y: Math.max(0, y - 16), animated: true });
  }, []);

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
        ref={scrollRef}
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator
        testID="skill-library-page"
      >
        <View style={styles.hero}>
          <Text style={styles.title}>{text.skills}</Text>
          <Text style={styles.subtitle}>
            {locale === "zh"
              ? "本地技能 · Claude / Codex · SkillsMP"
              : "Local skills · Claude / Codex · SkillsMP"}
          </Text>
          <View style={styles.heroSearchBox}>
            <Search size={20} color={theme.colors.foregroundMuted} />
            <TextInput
              value={searchQuery}
              onChangeText={setSearchQuery}
              placeholder={text.skillSearchPlaceholder}
              placeholderTextColor={theme.colors.foregroundMuted}
              style={[styles.searchInput, isWeb && ({ outlineStyle: "none" } as any)]}
              testID="skill-library-search"
            />
          </View>
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
          <Text style={styles.sectionTitle}>{locale === "zh" ? "本地技能" : "Local skills"}</Text>
          <Text style={styles.sectionCount}>{localSkillSummaries.length}</Text>
        </View>

        <View style={styles.skillGrid}>
          {localSkillSummaries.map((skill) => (
            <View key={skill.key} style={styles.skillRow} testID={`local-skill-row-${skill.key}`}>
              <View style={styles.skillIcon}>
                <Box size={22} color={theme.colors.accentBright} />
              </View>
              <View style={styles.skillCopy}>
                <View style={styles.skillTitleRow}>
                  <Text style={styles.skillName} numberOfLines={1}>
                    {skill.name}
                  </Text>
                  <View style={styles.installedBadge}>
                    <Check size={12} color={theme.colors.success} />
                    <Text style={styles.installedText}>
                      {skill.readOnly ? text.skillBuiltIn : text.skillInstalled}
                    </Text>
                  </View>
                </View>
                {skill.description ? (
                  <Text style={styles.skillDescription} numberOfLines={2}>
                    {skill.description}
                  </Text>
                ) : null}
                <View style={styles.pills}>
                  <View style={styles.pill}>
                    <Text style={styles.pillText}>
                      {skill.scope === "workspace"
                        ? locale === "zh"
                          ? "工作区"
                          : "Workspace"
                        : locale === "zh"
                          ? "全局"
                          : "Global"}
                    </Text>
                  </View>
                  {skill.sources.map((source) => (
                    <View key={source} style={styles.pill}>
                      <Text style={styles.pillText}>{localSkillSourceLabel(source, locale)}</Text>
                    </View>
                  ))}
                </View>
                <Text style={styles.skillSource} numberOfLines={1}>
                  {skill.path}
                </Text>
              </View>
            </View>
          ))}
          {localSkillSummaries.length === 0 ? (
            <Text style={styles.emptyText}>
              {locale === "zh" ? "未找到本地技能" : "No local skills found"}
            </Text>
          ) : null}
        </View>

        <View style={styles.sectionHeader}>
          <Text style={styles.sectionTitle}>
            {locale === "zh" ? "市场技能" : "Marketplace skills"}
          </Text>
          <Text style={styles.sectionCount}>{marketplaceCountLabel}</Text>
        </View>

        {showMarketplaceAlphabet ? (
          <View style={styles.marketplaceGroups}>
            {marketplaceGroups.map((group) => (
              <View
                key={group.letter}
                style={styles.marketplaceLetterSection}
                testID={`skill-marketplace-letter-section-${group.letter}`}
                onLayout={(event) => {
                  marketplaceLetterOffsetsRef.current.set(group.letter, event.nativeEvent.layout.y);
                }}
              >
                <View style={styles.marketplaceLetterHeader}>
                  <Text style={styles.marketplaceLetterText}>{group.letter}</Text>
                  <Text style={styles.sectionCount}>{group.skills.length}</Text>
                </View>
                <View style={styles.skillGrid}>{group.skills.map(renderMarketplaceSkillRow)}</View>
              </View>
            ))}
          </View>
        ) : (
          <View style={styles.skillGrid}>{marketplaceSkills.map(renderMarketplaceSkillRow)}</View>
        )}
      </ScrollView>
      {showMarketplaceAlphabet ? (
        <View style={styles.alphabetRail} testID="skill-marketplace-alphabet-rail">
          {MARKETPLACE_ALPHABET.map((letter) => {
            const enabled = marketplaceLettersWithResults.has(letter);
            return (
              <Pressable
                key={letter}
                style={[styles.alphabetButton, !enabled ? styles.alphabetButtonDisabled : null]}
                accessibilityRole="button"
                disabled={!enabled}
                testID={`skill-marketplace-letter-${letter}`}
                onPress={() => scrollToMarketplaceLetter(letter)}
              >
                <Text
                  style={[styles.alphabetButtonText, !enabled ? styles.disabledText : null]}
                  numberOfLines={1}
                >
                  {letter}
                </Text>
              </Pressable>
            );
          })}
        </View>
      ) : null}
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
  heroSearchBox: {
    width: "100%",
    maxWidth: 680,
    minHeight: 50,
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    marginTop: theme.spacing[6],
    paddingHorizontal: theme.spacing[4],
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
    alignItems: "center",
    gap: theme.spacing[2],
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize["4xl"],
    fontWeight: theme.fontWeight.normal,
    textAlign: "center",
  },
  subtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.lg,
    textAlign: "center",
  },
  workspaceHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    flex: 1,
    minWidth: 0,
  },
  workspacePickerWrap: {
    position: "relative",
    alignSelf: "center",
    width: "100%",
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
  marketplaceGroups: {
    gap: theme.spacing[10],
  },
  marketplaceLetterSection: {
    gap: theme.spacing[4],
  },
  marketplaceLetterHeader: {
    minHeight: 30,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  marketplaceLetterText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.semibold,
  },
  skillGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    columnGap: 40,
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
    paddingBottom: 20,
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
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
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
  alphabetRail: {
    position: "absolute",
    right: theme.spacing[2],
    top: 112,
    bottom: theme.spacing[8],
    width: 28,
    alignItems: "center",
    justifyContent: "center",
    gap: 1,
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  alphabetButton: {
    width: 24,
    minHeight: 18,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.sm,
  },
  alphabetButtonDisabled: {
    opacity: 0.35,
  },
  alphabetButtonText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.semibold,
  },
}));
