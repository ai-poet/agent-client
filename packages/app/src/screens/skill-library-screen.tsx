import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Modal, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import {
  Box,
  Check,
  ChevronDown,
  Edit3,
  Eye,
  ExternalLink,
  FileArchive,
  Plus,
  RefreshCcw,
  Save,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import type {
  ContextHubManagedSkillEntry,
  ContextHubMarketplaceSkillEntry,
  ContextHubSkillScope,
  ContextHubSkillWritableTarget,
} from "@server/shared/messages";
import { MenuHeader } from "@/components/headers/menu-header";
import { isWeb } from "@/constants/platform";
import { useToast } from "@/contexts/toast-context";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useHostRuntimeClient } from "@/runtime/host-runtime";
import { useNavigationActiveWorkspaceSelection } from "@/stores/navigation-active-workspace-store";
import { useWorkspaceList } from "@/stores/session-store-hooks";
import { confirmDialog } from "@/utils/confirm-dialog";
import { toErrorMessage } from "@/utils/error-messages";
import { openExternalUrl } from "@/utils/open-external-url";
import { pickSkillZipBase64 } from "@/utils/pick-skill-zip";

function normalizeSearch(value: string): string {
  return value.trim();
}

const MARKETPLACE_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");
const DEFAULT_SKILL_CONTENT =
  "---\nname: my-skill\ndescription: Describe when to use this skill.\n---\n\n# My skill\n\nUse this skill when...\n";

type SkillLibraryText = ReturnType<typeof getSub2APIMessages>["sidebar"];

type SkillTargetOption = {
  value: ContextHubSkillWritableTarget;
  scope: ContextHubSkillScope;
  label: string;
};

type LocalSkillSelection =
  | {
      mode: "existing";
      skillId: string;
    }
  | {
      mode: "new";
    };

function skillTargetOptions(locale: string, hasWorkspace: boolean): SkillTargetOption[] {
  const isZh = locale === "zh";
  return [
    { value: "managed", scope: "global", label: isZh ? "Paseo 管理" : "Paseo managed" },
    { value: "global_codex", scope: "global", label: isZh ? "全局 Codex" : "Global Codex" },
    { value: "global_claude", scope: "global", label: isZh ? "全局 Claude" : "Global Claude" },
    { value: "global_agents", scope: "global", label: isZh ? "全局 Agents" : "Global Agents" },
    ...(hasWorkspace
      ? [
          {
            value: "project_codex" as const,
            scope: "workspace" as const,
            label: isZh ? "工作区 Codex" : "Workspace Codex",
          },
          {
            value: "project_claude" as const,
            scope: "workspace" as const,
            label: isZh ? "工作区 Claude" : "Workspace Claude",
          },
          {
            value: "project_agents" as const,
            scope: "workspace" as const,
            label: isZh ? "工作区 Agents" : "Workspace Agents",
          },
        ]
      : []),
  ];
}

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

function permissionLabels(
  skill: ContextHubMarketplaceSkillEntry,
  text: SkillLibraryText,
): string[] {
  const labels: string[] = [];
  if (skill.permissions.network) labels.push(text.skillPermissions.network);
  if (skill.permissions.filesystem) labels.push(text.skillPermissions.filesystem);
  if (skill.permissions.subprocess) labels.push(text.skillPermissions.subprocess);
  if (skill.permissions.envVars.length > 0) {
    labels.push(text.skillPermissions.envVars(skill.permissions.envVars.length));
  }
  return labels.length > 0 ? labels : [text.skillPermissions.none];
}

function skillStats(skill: ContextHubMarketplaceSkillEntry, text: SkillLibraryText): string {
  const age =
    skill.daysSinceUpdate === null
      ? text.skillStats.updatedRecently
      : text.skillStats.daysSinceUpdate(skill.daysSinceUpdate);
  return text.skillStats.summary(skill.trustLevel, skill.downloadCount, skill.downloads7d, age);
}

function marketplaceSkillInitial(name: string): string | null {
  const initial = name
    .trim()
    .match(/[A-Za-z]/)?.[0]
    ?.toUpperCase();
  return initial && /^[A-Z]$/.test(initial) ? initial : null;
}

function groupMarketplaceSkills(skills: ContextHubMarketplaceSkillEntry[]): Array<{
  letter: string;
  skills: ContextHubMarketplaceSkillEntry[];
}> {
  const buckets = new Map<string, ContextHubMarketplaceSkillEntry[]>();
  for (const letter of MARKETPLACE_ALPHABET) buckets.set(letter, []);
  for (const skill of skills) {
    const initial = marketplaceSkillInitial(skill.name);
    if (!initial) continue;
    buckets.get(initial)?.push(skill);
  }
  return MARKETPLACE_ALPHABET.map((letter) => ({
    letter,
    skills: buckets.get(letter) ?? [],
  })).filter((group) => group.skills.length > 0);
}

function marketplaceSkillUrl(skill: ContextHubMarketplaceSkillEntry): string | null {
  const prefix = "skillsmp:";
  if (!skill.id.startsWith(prefix)) return null;
  const slug = skill.id.slice(prefix.length).trim();
  if (!slug || slug.includes("/") || slug.includes("..")) return null;
  return `https://skillsmp.com/skills/${encodeURIComponent(slug)}`;
}

function skillNameFromContent(content: string): string | null {
  const frontmatter = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  const fromFrontMatter = frontmatter?.[1].match(/^name:\s*(.+)$/m)?.[1]?.trim();
  if (fromFrontMatter) return fromFrontMatter.replace(/^["']|["']$/g, "");
  const heading = content.match(/^#\s+(.+)$/m)?.[1]?.trim();
  return heading || null;
}

function safeDraftName(value: string): string {
  return (
    value
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 80) || "skill"
  );
}

function SkillSkeletonRow({ width = "47%" as const }: { width?: "47%" | "100%" }) {
  return (
    <View style={[styles.skillRow, { width }]} testID="skill-library-skeleton-row">
      <View style={[styles.skillIcon, styles.skeletonBlock]} />
      <View style={styles.skillCopy}>
        <View style={[styles.skeletonLine, styles.skeletonTitle]} />
        <View style={[styles.skeletonLine, styles.skeletonDescription]} />
        <View style={styles.pills}>
          <View style={[styles.skeletonPill, styles.skeletonBlock]} />
          <View style={[styles.skeletonPill, styles.skeletonBlock]} />
        </View>
        <View style={[styles.skeletonLine, styles.skeletonMeta]} />
      </View>
    </View>
  );
}

function SkillSectionSkeleton() {
  return (
    <View style={styles.skillGrid} testID="skill-library-skeleton">
      <SkillSkeletonRow />
      <SkillSkeletonRow />
      <SkillSkeletonRow />
      <SkillSkeletonRow />
    </View>
  );
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
  const messages = useMemo(() => getSub2APIMessages(locale), [locale]);
  const text = messages.sidebar;
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
  const [isSaving, setIsSaving] = useState(false);
  const [isEditing, setIsEditing] = useState(false);
  const [selection, setSelection] = useState<LocalSkillSelection | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftContent, setDraftContent] = useState(DEFAULT_SKILL_CONTENT);
  const [selectedTarget, setSelectedTarget] = useState<ContextHubSkillWritableTarget>("managed");

  useEffect(() => {
    if (activeWorkspaceId) setSelectedWorkspaceId(activeWorkspaceId);
  }, [activeWorkspaceId]);

  useEffect(() => {
    if (!selectedWorkspaceId && workspaces[0]) setSelectedWorkspaceId(workspaces[0].id);
  }, [selectedWorkspaceId, workspaces]);

  const workspace = useMemo(
    () => workspaces.find((entry) => entry.id === selectedWorkspaceId) ?? null,
    [selectedWorkspaceId, workspaces],
  );

  const targetOptions = useMemo(
    () => skillTargetOptions(locale, Boolean(workspace)),
    [locale, workspace],
  );
  const selectedTargetOption =
    targetOptions.find((option) => option.value === selectedTarget) ?? targetOptions[0];

  useEffect(() => {
    if (!targetOptions.some((option) => option.value === selectedTarget)) {
      setSelectedTarget(targetOptions[0]?.value ?? "managed");
    }
  }, [selectedTarget, targetOptions]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(normalizeSearch(searchQuery)), 250);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  const refresh = useCallback(async () => {
    if (!client) {
      setLocalSkills([]);
      setMarketplaceSkills([]);
      setErrorMessage(messages.common.hostNotConnected);
      return;
    }
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const [localResult, marketplaceResult] = await Promise.allSettled([
        client.skillsList({
          workspaceId: workspace?.id,
          cwd: workspace?.workspaceDirectory,
          includeContent: true,
        }),
        client.skillsMarketplaceList({
          query: debouncedQuery || undefined,
          limit: 50,
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
  }, [
    client,
    debouncedQuery,
    messages.common.hostNotConnected,
    workspace?.id,
    workspace?.workspaceDirectory,
  ]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const filteredLocalSkills = useMemo(() => {
    const query = debouncedQuery.toLowerCase();
    return localSkills
      .filter((skill) => {
        if (!query) return true;
        return [skill.name, skill.description, skill.source, skill.path]
          .filter(Boolean)
          .join("\n")
          .toLowerCase()
          .includes(query);
      })
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [debouncedQuery, localSkills]);

  const selectedSkill =
    selection?.mode === "existing"
      ? (localSkills.find((skill) => skill.id === selection.skillId) ?? null)
      : null;
  const canMutateSelectedSkill =
    Boolean(selectedSkill) &&
    selectedSkill?.readOnly === false &&
    selectedSkill.source !== "bundled";

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
    ? text.skillMarketplaceAtoZ(marketplaceSkills.length)
    : String(marketplaceSkills.length);

  const openLocalSkill = useCallback(
    (skill: ContextHubManagedSkillEntry, edit = false) => {
      setSelection({ mode: "existing", skillId: skill.id });
      setDraftName(skill.name);
      setDraftContent(skill.content ?? "");
      if (skill.source !== "bundled" && skill.source !== selectedTarget) {
        setSelectedTarget(skill.source as ContextHubSkillWritableTarget);
      }
      setIsEditing(edit && !skill.readOnly);
    },
    [selectedTarget],
  );

  const createNewSkill = useCallback(() => {
    const defaultName = "my-skill";
    setSelection({ mode: "new" });
    setDraftName(defaultName);
    setDraftContent(DEFAULT_SKILL_CONTENT);
    setIsEditing(true);
  }, []);

  const saveSkill = useCallback(
    async (overwrite = false) => {
      if (!client || !selectedTargetOption) return;
      if (selectedTargetOption.scope === "workspace" && !workspace) {
        toast.error(text.skillWorkspaceRequired);
        return;
      }
      const name = safeDraftName(draftName || skillNameFromContent(draftContent) || "skill");
      setIsSaving(true);
      try {
        const result = await client.skillsSave({
          target: selectedTargetOption.value,
          scope: selectedTargetOption.scope,
          skillId: selection?.mode === "existing" ? selection.skillId : undefined,
          name,
          content: draftContent,
          workspaceId: workspace?.id,
          cwd: workspace?.workspaceDirectory,
          overwrite,
        });
        if (result.error) {
          if (result.conflict && !overwrite) {
            toast.error(text.skillSaveConflict(name));
          } else {
            toast.error(result.error);
          }
          return;
        }
        if (result.skill) {
          setSelection({ mode: "existing", skillId: result.skill.id });
          setDraftName(result.skill.name);
          setDraftContent(result.skill.content ?? draftContent);
        }
        setIsEditing(false);
        toast.show(text.skillSaveSuccess(name), { variant: "success" });
        await refresh();
      } catch (error) {
        toast.error(toErrorMessage(error));
      } finally {
        setIsSaving(false);
      }
    },
    [
      client,
      draftContent,
      draftName,
      refresh,
      selectedTargetOption,
      selection,
      text,
      toast,
      workspace,
    ],
  );

  const importZip = useCallback(
    async (overwrite = false) => {
      if (!client || !selectedTargetOption) return;
      if (selectedTargetOption.scope === "workspace" && !workspace) {
        toast.error(text.skillWorkspaceRequired);
        return;
      }
      setIsSaving(true);
      try {
        const picked = await pickSkillZipBase64();
        if (!picked) return;
        const name = safeDraftName(picked.name || draftName || "skill");
        const result = await client.skillsImportPackage({
          target: selectedTargetOption.value,
          scope: selectedTargetOption.scope,
          name,
          packageBase64: picked.base64,
          workspaceId: workspace?.id,
          cwd: workspace?.workspaceDirectory,
          overwrite,
        });
        if (result.error) {
          if (result.conflict && !overwrite) {
            toast.error(text.skillSaveConflict(name));
          } else {
            toast.error(result.error);
          }
          return;
        }
        if (result.skill) {
          setSelection({ mode: "existing", skillId: result.skill.id });
          setDraftName(result.skill.name);
          setDraftContent(result.skill.content ?? "");
          setIsEditing(false);
        }
        toast.show(text.skillImportZipSuccess(name), { variant: "success" });
        await refresh();
      } catch (error) {
        toast.error(toErrorMessage(error));
      } finally {
        setIsSaving(false);
      }
    },
    [client, draftName, refresh, selectedTargetOption, text, toast, workspace],
  );

  const closeEditor = useCallback(() => {
    setSelection(null);
    setDraftName("");
    setDraftContent("");
    setIsEditing(false);
  }, []);

  const deleteSkill = useCallback(async () => {
    if (!client || !selectedSkill) return;
    const confirmed = await confirmDialog({
      title: text.skillDeleteConfirmTitle,
      message: text.skillDeleteConfirmMessage(selectedSkill.name),
      confirmLabel: text.skillDelete,
      cancelLabel: messages.common.cancel,
      destructive: true,
    });
    if (!confirmed) return;
    setIsSaving(true);
    try {
      await client.skillsDelete({
        skillId: selectedSkill.id,
        workspaceId: workspace?.id,
        cwd: workspace?.workspaceDirectory,
      });
      toast.show(text.skillDeleteSuccess(selectedSkill.name), { variant: "success" });
      closeEditor();
      setDraftName("");
      setDraftContent("");
      setIsEditing(false);
      await refresh();
    } catch (error) {
      toast.error(toErrorMessage(error));
    } finally {
      setIsSaving(false);
    }
  }, [client, closeEditor, messages.common.cancel, refresh, selectedSkill, text, toast, workspace]);

  const openMarketplaceSkill = useCallback(
    async (skill: ContextHubMarketplaceSkillEntry) => {
      const url = marketplaceSkillUrl(skill);
      if (!url) {
        toast.error(text.skillOpenFailed);
        return;
      }
      try {
        await openExternalUrl(url);
      } catch (error) {
        toast.error(toErrorMessage(error));
      }
    },
    [text.skillOpenFailed, toast],
  );

  const scrollToMarketplaceLetter = useCallback((letter: string) => {
    const y = marketplaceLetterOffsetsRef.current.get(letter);
    if (typeof y !== "number") return;
    scrollRef.current?.scrollTo({ y: Math.max(0, y - 16), animated: true });
  }, []);

  const renderMarketplaceSkillRow = useCallback(
    (skill: ContextHubMarketplaceSkillEntry) => {
      const canOpen = Boolean(marketplaceSkillUrl(skill));
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
              {permissionLabels(skill, text).map((label) => (
                <View key={label} style={styles.pill}>
                  <Text style={styles.pillText}>{label}</Text>
                </View>
              ))}
            </View>
            <Text style={styles.skillSource} numberOfLines={1}>
              {skillStats(skill, text)}
            </Text>
          </View>
          <Pressable
            style={[styles.installAction, !canOpen ? styles.installActionDisabled : null]}
            accessibilityRole="button"
            testID={`skill-marketplace-install-${skill.id}`}
            onPress={() => void openMarketplaceSkill(skill)}
            disabled={!canOpen}
          >
            <ExternalLink
              size={18}
              color={canOpen ? theme.colors.accentForeground : theme.colors.foregroundMuted}
            />
            <Text
              style={[
                styles.installActionText,
                { color: canOpen ? theme.colors.accentForeground : theme.colors.foregroundMuted },
              ]}
            >
              {text.skillOpen}
            </Text>
          </Pressable>
        </View>
      );
    },
    [openMarketplaceSkill, text, theme],
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
          {isLoading ? text.skillRefreshing : text.skillRefresh}
        </Text>
      </Pressable>
    </View>
  );

  const workspaceLabel = workspace
    ? `${workspace.projectDisplayName || workspace.name} · ${workspace.name}`
    : text.skillNoWorkspace;
  const editorTitle =
    selection?.mode === "new"
      ? text.skillNew
      : selectedSkill?.name
        ? selectedSkill.name
        : text.skillSelectSkill;

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
          <Text style={styles.subtitle}>{text.skillLibraryContentSubtitle}</Text>
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
                  <Text style={styles.workspacePickerEmpty}>{text.skillNoWorkspaces}</Text>
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

        <View style={styles.localManager} testID="skill-local-manager">
          <View style={styles.localListPane}>
            <View style={styles.sectionHeader}>
              <Text style={styles.sectionTitle}>{text.skillLocalSkills}</Text>
              <Text style={styles.sectionCount}>{filteredLocalSkills.length}</Text>
            </View>
            <View style={styles.targetWrap}>
              {targetOptions.map((option) => (
                <Pressable
                  key={option.value}
                  style={[
                    styles.targetButton,
                    selectedTarget === option.value ? styles.targetButtonActive : null,
                  ]}
                  accessibilityRole="button"
                  testID={`skill-target-${option.value}`}
                  onPress={() => setSelectedTarget(option.value)}
                >
                  <Text
                    style={[
                      styles.targetButtonText,
                      selectedTarget === option.value ? styles.targetButtonTextActive : null,
                    ]}
                    numberOfLines={1}
                  >
                    {option.label}
                  </Text>
                </Pressable>
              ))}
            </View>
            <View style={styles.localActions}>
              <Pressable
                style={styles.secondaryAction}
                accessibilityRole="button"
                testID="skill-new-button"
                onPress={createNewSkill}
              >
                <Plus size={16} color={theme.colors.foreground} />
                <Text style={styles.secondaryActionText}>{text.skillNew}</Text>
              </Pressable>
              <Pressable
                style={styles.secondaryAction}
                accessibilityRole="button"
                testID="skill-import-zip-button"
                disabled={isSaving}
                onPress={() => void importZip()}
              >
                <FileArchive size={16} color={theme.colors.foreground} />
                <Text style={styles.secondaryActionText}>{text.skillImportZip}</Text>
              </Pressable>
            </View>
            {isLoading && filteredLocalSkills.length === 0 ? (
              <SkillSectionSkeleton />
            ) : (
              <View style={styles.localSkillList}>
                {filteredLocalSkills.map((skill) => (
                  <View
                    key={skill.id}
                    style={styles.localSkillItem}
                    testID={`local-skill-row-${skill.name.toLowerCase()}`}
                  >
                    <View style={styles.localSkillHeader}>
                      <View style={styles.localSkillHeading}>
                        <Text style={styles.skillName} numberOfLines={1}>
                          {skill.name}
                        </Text>
                        <View style={styles.installedBadge}>
                          {skill.readOnly ? null : <Check size={12} color={theme.colors.success} />}
                          <Text style={styles.installedText}>
                            {skill.readOnly ? text.skillBuiltIn : text.skillEditable}
                          </Text>
                        </View>
                      </View>
                      <Pressable
                        style={styles.iconAction}
                        accessibilityRole="button"
                        testID={`local-skill-edit-${skill.id}`}
                        onPress={() => openLocalSkill(skill, !skill.readOnly)}
                      >
                        {skill.readOnly ? (
                          <Eye size={16} color={theme.colors.foreground} />
                        ) : (
                          <Edit3 size={16} color={theme.colors.foreground} />
                        )}
                      </Pressable>
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
                            ? text.skillScopeWorkspace
                            : text.skillScopeGlobal}
                        </Text>
                      </View>
                      <View style={styles.pill}>
                        <Text style={styles.pillText}>
                          {localSkillSourceLabel(skill.source, locale)}
                        </Text>
                      </View>
                    </View>
                    <Text style={styles.skillSource} numberOfLines={1}>
                      {skill.path}
                    </Text>
                  </View>
                ))}
                {filteredLocalSkills.length === 0 ? (
                  <Text style={styles.emptyText}>{text.skillNoLocalSkills}</Text>
                ) : null}
              </View>
            )}
          </View>
        </View>

        <View style={styles.sectionHeader}>
          <Text style={styles.sectionTitle}>{text.skillMarketplaceSkills}</Text>
          <Text style={styles.sectionCount}>{marketplaceCountLabel}</Text>
        </View>

        {isLoading && marketplaceSkills.length === 0 ? (
          <SkillSectionSkeleton />
        ) : showMarketplaceAlphabet ? (
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
      <Modal
        transparent
        animationType="fade"
        visible={selection !== null}
        onRequestClose={closeEditor}
      >
        <View style={styles.modalBackdrop}>
          <View style={styles.editorPane} testID="skill-editor-pane">
            <View style={styles.editorHeader}>
              <View style={styles.editorTitleWrap}>
                <Text style={styles.sectionTitle} numberOfLines={1}>
                  {editorTitle}
                </Text>
                {selectedSkill ? (
                  <Text style={styles.skillSource} numberOfLines={1}>
                    {selectedSkill.path}
                  </Text>
                ) : null}
              </View>
              <View style={styles.editorActions}>
                {selection && selectedSkill?.readOnly !== true ? (
                  <Pressable
                    style={styles.iconAction}
                    accessibilityRole="button"
                    testID="skill-edit-toggle"
                    onPress={() => setIsEditing((value) => !value)}
                  >
                    {isEditing ? (
                      <X size={16} color={theme.colors.foregroundMuted} />
                    ) : (
                      <Edit3 size={16} color={theme.colors.foreground} />
                    )}
                  </Pressable>
                ) : null}
                {canMutateSelectedSkill ? (
                  <Pressable
                    style={styles.iconAction}
                    accessibilityRole="button"
                    testID="skill-delete-button"
                    disabled={isSaving}
                    onPress={() => void deleteSkill()}
                  >
                    <Trash2 size={16} color={theme.colors.foreground} />
                  </Pressable>
                ) : null}
                <Pressable
                  style={styles.iconAction}
                  accessibilityRole="button"
                  testID="skill-editor-close"
                  onPress={closeEditor}
                >
                  <X size={16} color={theme.colors.foregroundMuted} />
                </Pressable>
              </View>
            </View>
            {selection ? (
              <View style={styles.editorBody}>
                <Text style={styles.fieldLabel}>{text.skillNameLabel}</Text>
                <TextInput
                  value={draftName}
                  onChangeText={setDraftName}
                  editable={isEditing && selection.mode === "new"}
                  placeholder="my-skill"
                  placeholderTextColor={theme.colors.foregroundMuted}
                  style={[styles.fieldInput, isWeb && ({ outlineStyle: "none" } as any)]}
                  testID="skill-name-input"
                />
                <Text style={styles.fieldLabel}>SKILL.md</Text>
                <TextInput
                  value={draftContent}
                  onChangeText={setDraftContent}
                  editable={isEditing && (selection.mode === "new" || canMutateSelectedSkill)}
                  multiline
                  textAlignVertical="top"
                  placeholder={DEFAULT_SKILL_CONTENT}
                  placeholderTextColor={theme.colors.foregroundMuted}
                  style={[styles.codeInput, isWeb && ({ outlineStyle: "none" } as any)]}
                  testID="skill-content-input"
                />
                {isEditing ? (
                  <View style={styles.editorFooter}>
                    <Pressable
                      style={[styles.installAction, isSaving ? styles.installActionDisabled : null]}
                      accessibilityRole="button"
                      testID="skill-save-button"
                      disabled={isSaving}
                      onPress={() => void saveSkill()}
                    >
                      <Save
                        size={18}
                        color={
                          isSaving ? theme.colors.foregroundMuted : theme.colors.accentForeground
                        }
                      />
                      <Text
                        style={[
                          styles.installActionText,
                          {
                            color: isSaving
                              ? theme.colors.foregroundMuted
                              : theme.colors.accentForeground,
                          },
                        ]}
                      >
                        {isSaving ? text.skillSaving : text.skillSave}
                      </Text>
                    </Pressable>
                  </View>
                ) : null}
              </View>
            ) : null}
          </View>
        </View>
      </Modal>
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
  localManager: {
    gap: theme.spacing[4],
  },
  localListPane: {
    gap: theme.spacing[4],
  },
  modalBackdrop: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: theme.spacing[6],
    backgroundColor: "rgba(0, 0, 0, 0.44)",
  },
  editorPane: {
    width: "100%",
    maxWidth: 920,
    maxHeight: "88%",
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface1,
    overflow: "hidden",
  },
  targetWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  targetButton: {
    minHeight: 30,
    maxWidth: 180,
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[3],
    backgroundColor: theme.colors.surface1,
  },
  targetButtonActive: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.accent,
  },
  targetButtonText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  targetButtonTextActive: {
    color: theme.colors.accentForeground,
    fontWeight: theme.fontWeight.semibold,
  },
  localActions: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  secondaryAction: {
    minHeight: 34,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[3],
    backgroundColor: theme.colors.surface1,
  },
  secondaryActionText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
  },
  localSkillList: {
    flexDirection: "row",
    flexWrap: "wrap",
    columnGap: theme.spacing[4],
    rowGap: theme.spacing[4],
  },
  localSkillHeader: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  localSkillHeading: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  localSkillItem: {
    width: "47%",
    minWidth: 360,
    minHeight: 118,
    gap: theme.spacing[2],
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    padding: theme.spacing[3],
    backgroundColor: theme.colors.surface1,
  },
  editorHeader: {
    minHeight: 58,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    paddingHorizontal: theme.spacing[4],
  },
  editorTitleWrap: {
    flex: 1,
    minWidth: 0,
  },
  editorActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  iconAction: {
    width: 34,
    height: 34,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
  },
  editorBody: {
    flex: 1,
    gap: theme.spacing[3],
    padding: theme.spacing[4],
  },
  editorFooter: {
    flexDirection: "row",
    justifyContent: "flex-end",
  },
  fieldLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.semibold,
  },
  fieldInput: {
    minHeight: 40,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[3],
    color: theme.colors.foreground,
    backgroundColor: theme.colors.surface0,
    fontSize: theme.fontSize.sm,
  },
  codeInput: {
    minHeight: 320,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    padding: theme.spacing[3],
    color: theme.colors.foreground,
    backgroundColor: theme.colors.surface0,
    fontSize: theme.fontSize.sm,
    lineHeight: 20,
  },
  marketplaceGroups: {
    gap: theme.spacing[8],
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
  skeletonBlock: {
    backgroundColor: theme.colors.surface2,
  },
  skeletonLine: {
    height: 12,
    borderRadius: theme.borderRadius.sm,
    backgroundColor: theme.colors.surface2,
  },
  skeletonTitle: {
    width: "48%",
    height: 16,
  },
  skeletonDescription: {
    width: "86%",
  },
  skeletonMeta: {
    width: "64%",
    height: 10,
  },
  skeletonPill: {
    width: 76,
    height: 24,
    borderRadius: theme.borderRadius.sm,
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
    backgroundColor: theme.colors.accent,
  },
  installActionDisabled: {
    backgroundColor: theme.colors.surface1,
  },
  installActionText: {
    color: theme.colors.accentForeground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
  },
  disabledText: {
    color: theme.colors.foregroundMuted,
  },
  alphabetRail: {
    position: "absolute",
    right: theme.spacing[2],
    top: 132,
    bottom: theme.spacing[8],
    justifyContent: "center",
    gap: 1,
  },
  alphabetButton: {
    width: 24,
    height: 20,
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
