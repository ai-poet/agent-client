import { useCallback, useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, ScrollView, Text, View } from "react-native";
import { router } from "expo-router";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Check, RotateCcw, Save, Users } from "lucide-react-native";
import {
  createDefaultWorktreePersona,
  getWorktreePersonaRole,
  getWorktreePersonaSkill,
  WORKTREE_PERSONA_ROLES,
  WORKTREE_PERSONA_SKILLS,
  type WorktreePersona,
  type WorktreePersonaRoleDefinition,
  type WorktreePersonaRoleId,
  type WorktreePersonaSkillDefinition,
} from "@server/shared/worktree-persona";
import { MenuHeader } from "@/components/headers/menu-header";
import { useToast } from "@/contexts/toast-context";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getHostRuntimeStore } from "@/runtime/host-runtime";
import { normalizeWorkspaceDescriptor, useSessionStore } from "@/stores/session-store";
import { useWorkspace } from "@/stores/session-store-hooks";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { buildHostWorkspaceRoute } from "@/utils/host-routes";

interface WorkspaceColleagueScreenProps {
  serverId: string;
  workspaceId: string;
}

function arePersonasEqual(left: WorktreePersona, right: WorktreePersona): boolean {
  return left.roleId === right.roleId && left.skillIds.join("\0") === right.skillIds.join("\0");
}

function roleLabel(role: WorktreePersonaRoleDefinition, locale: "zh" | "en"): string {
  return locale === "zh" ? role.labelZh : role.label;
}

function roleDescription(role: WorktreePersonaRoleDefinition, locale: "zh" | "en"): string {
  return locale === "zh" ? role.descriptionZh : role.description;
}

function formatSkillNames(skillIds: string[]): string {
  return skillIds.map((skillId) => getWorktreePersonaSkill(skillId)?.name ?? skillId).join(", ");
}

export function WorkspaceColleagueScreen({ serverId, workspaceId }: WorkspaceColleagueScreenProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const agentFormText = useMemo(() => getSub2APIMessages(locale).agentForm, [locale]);
  const toast = useToast();
  const workspace = useWorkspace(serverId || null, workspaceId || null);
  const workspacePersona = workspace?.worktreePersona ?? null;
  const [draft, setDraft] = useState<WorktreePersona>(
    () => workspacePersona ?? createDefaultWorktreePersona(),
  );
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    setDraft(workspacePersona ?? createDefaultWorktreePersona());
  }, [workspacePersona]);

  const selectedRole = getWorktreePersonaRole(draft.roleId);
  const isWorktree = workspace?.workspaceKind === "worktree";
  const isDirty = !arePersonasEqual(draft, workspacePersona ?? createDefaultWorktreePersona());
  const selectedSkillNames = formatSkillNames(draft.skillIds);

  const handleSelectRole = useCallback((roleId: WorktreePersonaRoleId) => {
    const role = getWorktreePersonaRole(roleId);
    setDraft({
      roleId: role.id,
      skillIds: role.defaultSkillIds,
    });
  }, []);

  const handleToggleSkill = useCallback((skillId: string) => {
    setDraft((current) => {
      const hasSkill = current.skillIds.includes(skillId);
      const skillIds = hasSkill
        ? current.skillIds.filter((id) => id !== skillId)
        : [...current.skillIds, skillId];
      return {
        ...current,
        skillIds,
      };
    });
  }, []);

  const handleReset = useCallback(() => {
    setDraft(workspacePersona ?? createDefaultWorktreePersona());
  }, [workspacePersona]);

  const handleSave = useCallback(() => {
    if (isSaving || !workspace) {
      return;
    }
    const client = getHostRuntimeStore().getClient(serverId);
    if (!client) {
      toast.error(text.hostNotConnected);
      return;
    }

    setIsSaving(true);
    void client
      .updateWorkspacePersona({
        workspaceId: workspace.id,
        worktreePersona: draft,
      })
      .then((payload) => {
        if (payload.error || !payload.workspace) {
          throw new Error(payload.error ?? text.failedConfigureColleague);
        }
        useSessionStore
          .getState()
          .mergeWorkspaces(serverId, [normalizeWorkspaceDescriptor(payload.workspace)]);
        toast.show(text.colleagueSaved, { variant: "success" });
        router.replace(buildHostWorkspaceRoute(serverId, workspace.id));
      })
      .catch((error) => {
        toast.error(error instanceof Error ? error.message : text.failedConfigureColleague);
      })
      .finally(() => {
        setIsSaving(false);
      });
  }, [draft, isSaving, serverId, text, toast, workspace]);

  const headerActions = (
    <View style={styles.headerActions}>
      <Pressable
        accessibilityRole="button"
        disabled={!isDirty || isSaving || !isWorktree}
        onPress={handleReset}
        style={({ hovered, pressed }) => [
          styles.secondaryAction,
          (hovered || pressed) && styles.secondaryActionHovered,
          (!isDirty || isSaving || !isWorktree) && styles.actionDisabled,
        ]}
        testID="workspace-colleague-reset"
      >
        <RotateCcw size={16} color={theme.colors.foregroundMuted} />
        <Text style={styles.secondaryActionText}>{text.resetColleague}</Text>
      </Pressable>
      <Pressable
        accessibilityRole="button"
        disabled={!isDirty || isSaving || !isWorktree}
        onPress={handleSave}
        style={({ pressed }) => [
          styles.primaryAction,
          pressed && isDirty && !isSaving && isWorktree && styles.primaryActionPressed,
          (!isDirty || isSaving || !isWorktree) && styles.actionDisabled,
        ]}
        testID="workspace-colleague-save"
      >
        {isSaving ? (
          <ActivityIndicator size="small" color={theme.colors.accentForeground} />
        ) : (
          <Save size={16} color={theme.colors.accentForeground} />
        )}
        <Text style={styles.primaryActionText}>
          {isSaving ? text.savingColleague : text.saveColleague}
        </Text>
      </Pressable>
    </View>
  );

  if (!workspace) {
    return (
      <View style={styles.container}>
        <MenuHeader title={text.configureColleague} borderless />
        <View style={styles.emptyState}>
          <Text style={styles.emptyTitle}>{text.workspaceNotFound}</Text>
        </View>
      </View>
    );
  }

  if (!isWorktree) {
    return (
      <View style={styles.container}>
        <MenuHeader title={text.configureColleague} borderless />
        <View style={styles.emptyState}>
          <Text style={styles.emptyTitle}>{text.colleagueWorktreeOnly}</Text>
          <Text style={styles.emptyBody}>{workspace.name}</Text>
        </View>
      </View>
    );
  }

  return (
    <View style={styles.container}>
      <MenuHeader title={text.configureColleague} rightContent={headerActions} borderless />
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator
        testID="workspace-colleague-page"
      >
        <View style={styles.hero}>
          <View style={styles.avatar}>
            <Users size={28} color={theme.colors.foreground} />
          </View>
          <View style={styles.heroCopy}>
            <Text style={styles.title}>{workspace.name}</Text>
            <Text style={styles.subtitle}>
              {roleLabel(selectedRole, locale)} · {draft.skillIds.length} {agentFormText.skills}
            </Text>
            <Text style={styles.description}>{roleDescription(selectedRole, locale)}</Text>
          </View>
        </View>

        <View style={styles.summaryStrip}>
          <View style={styles.summaryItem}>
            <Text style={styles.summaryLabel}>{agentFormText.colleagueRole}</Text>
            <Text style={styles.summaryValue}>{roleLabel(selectedRole, locale)}</Text>
          </View>
          <View style={styles.summaryItem}>
            <Text style={styles.summaryLabel}>{agentFormText.skills}</Text>
            <Text style={styles.summaryValue} numberOfLines={1}>
              {selectedSkillNames || agentFormText.noSkillsSelected}
            </Text>
          </View>
          <View style={styles.summaryItem}>
            <Text style={styles.summaryLabel}>{text.path}</Text>
            <Text style={styles.summaryValue} numberOfLines={1}>
              {workspace.workspaceDirectory}
            </Text>
          </View>
        </View>

        <View style={styles.section}>
          <View style={styles.sectionHeader}>
            <Text style={styles.sectionTitle}>{agentFormText.colleagueRole}</Text>
            <Text style={styles.sectionHint}>{text.rolePickerHint}</Text>
          </View>
          <View style={styles.roleGrid}>
            {WORKTREE_PERSONA_ROLES.map((role) => {
              const selected = role.id === draft.roleId;
              return (
                <Pressable
                  key={role.id}
                  accessibilityRole="button"
                  accessibilityState={{ selected }}
                  onPress={() => handleSelectRole(role.id)}
                  style={({ hovered, pressed }) => [
                    styles.roleCard,
                    selected && styles.optionSelected,
                    (hovered || pressed) && styles.optionHovered,
                  ]}
                  testID={`workspace-colleague-role-${role.id}`}
                >
                  <View style={styles.optionTitleRow}>
                    <Text style={styles.roleTitle}>{roleLabel(role, locale)}</Text>
                    {selected ? <Check size={16} color={theme.colors.accentBright} /> : null}
                  </View>
                  <Text style={styles.roleDescription}>{roleDescription(role, locale)}</Text>
                  <Text style={styles.roleSkills} numberOfLines={1}>
                    {formatSkillNames(role.defaultSkillIds)}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>

        <View style={styles.section}>
          <View style={styles.sectionHeader}>
            <Text style={styles.sectionTitle}>{agentFormText.skills}</Text>
            <Text style={styles.sectionHint}>{agentFormText.skillsHelper}</Text>
          </View>
          <View style={styles.skillGrid}>
            {WORKTREE_PERSONA_SKILLS.map((skill) => (
              <SkillOption
                key={skill.id}
                skill={skill}
                selected={draft.skillIds.includes(skill.id)}
                onPress={() => handleToggleSkill(skill.id)}
              />
            ))}
          </View>
        </View>
      </ScrollView>
    </View>
  );
}

function SkillOption({
  skill,
  selected,
  onPress,
}: {
  skill: WorktreePersonaSkillDefinition;
  selected: boolean;
  onPress: () => void;
}) {
  const { theme } = useUnistyles();

  return (
    <Pressable
      accessibilityRole="checkbox"
      accessibilityState={{ checked: selected }}
      onPress={onPress}
      style={({ hovered, pressed }) => [
        styles.skillRow,
        selected && styles.optionSelected,
        (hovered || pressed) && styles.optionHovered,
      ]}
      testID={`workspace-colleague-skill-${skill.id}`}
    >
      <View style={styles.skillCheck}>
        {selected ? <Check size={14} color={theme.colors.accentBright} /> : null}
      </View>
      <View style={styles.skillCopy}>
        <Text style={styles.skillName}>{skill.name}</Text>
        <Text style={styles.skillDescription}>{skill.description}</Text>
        <Text style={styles.skillSource} numberOfLines={1}>
          {skill.sourceUrl} · {skill.license}
        </Text>
      </View>
    </Pressable>
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
    maxWidth: 1180,
    alignSelf: "center",
    paddingHorizontal: theme.spacing[8],
    paddingBottom: theme.spacing[16],
    gap: theme.spacing[8],
  },
  headerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  secondaryAction: {
    minHeight: 38,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
  },
  secondaryActionHovered: {
    backgroundColor: theme.colors.surface1,
  },
  secondaryActionText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  primaryAction: {
    minHeight: 38,
    minWidth: 108,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.accent,
  },
  primaryActionPressed: {
    opacity: 0.84,
  },
  primaryActionText: {
    color: theme.colors.accentForeground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
  },
  actionDisabled: {
    opacity: 0.5,
  },
  hero: {
    paddingTop: theme.spacing[12],
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[4],
  },
  avatar: {
    width: 72,
    height: 72,
    borderRadius: theme.borderRadius.full,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    flexShrink: 0,
  },
  heroCopy: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[2],
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize["3xl"],
    fontWeight: theme.fontWeight.semibold,
  },
  subtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.base,
  },
  description: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  summaryStrip: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  summaryItem: {
    minWidth: 220,
    flex: 1,
    gap: theme.spacing[1],
    paddingVertical: theme.spacing[3],
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
  },
  summaryLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    textTransform: "uppercase",
  },
  summaryValue: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
  section: {
    gap: theme.spacing[4],
  },
  sectionHeader: {
    gap: theme.spacing[1],
  },
  sectionTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.semibold,
  },
  sectionHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  roleGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[3],
  },
  roleCard: {
    width: "31%",
    minWidth: 220,
    minHeight: 132,
    gap: theme.spacing[3],
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  optionHovered: {
    borderColor: theme.colors.borderAccent,
    backgroundColor: theme.colors.surface2,
  },
  optionSelected: {
    borderColor: theme.colors.accentBright,
    backgroundColor: theme.colors.surface2,
  },
  optionTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
  },
  roleTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
  },
  roleDescription: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 20,
  },
  roleSkills: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  skillGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    columnGap: theme.spacing[4],
    rowGap: theme.spacing[3],
  },
  skillRow: {
    width: "48%",
    minWidth: 300,
    minHeight: 104,
    flexDirection: "row",
    gap: theme.spacing[3],
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  skillCheck: {
    width: 24,
    height: 24,
    borderRadius: theme.borderRadius.full,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.borderAccent,
    flexShrink: 0,
  },
  skillCopy: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
  },
  skillName: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
  },
  skillDescription: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 20,
  },
  skillSource: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  emptyState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    padding: theme.spacing[8],
  },
  emptyTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.semibold,
    textAlign: "center",
  },
  emptyBody: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
}));
