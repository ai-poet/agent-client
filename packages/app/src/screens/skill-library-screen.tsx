import { useMemo, useState } from "react";
import { ScrollView, Text, TextInput, View, Pressable } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Box, Check, Plus, RefreshCcw, Search } from "lucide-react-native";
import { WORKTREE_PERSONA_ROLES, WORKTREE_PERSONA_SKILLS } from "@server/shared/worktree-persona";
import { MenuHeader } from "@/components/headers/menu-header";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { isWeb } from "@/constants/platform";

function normalizeSearch(value: string): string {
  return value.trim().toLowerCase();
}

export function SkillLibraryScreen() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebar, [locale]);
  const [searchQuery, setSearchQuery] = useState("");
  const normalizedQuery = normalizeSearch(searchQuery);
  const isZh = locale === "zh";

  const skills = useMemo(
    () =>
      WORKTREE_PERSONA_SKILLS.map((skill) => ({
        skill,
        roles: WORKTREE_PERSONA_ROLES.filter((role) => role.defaultSkillIds.includes(skill.id)),
      })),
    [],
  );

  const filteredSkills = useMemo(() => {
    if (!normalizedQuery) {
      return skills;
    }
    return skills.filter(({ skill, roles }) => {
      const haystack = [
        skill.name,
        skill.description,
        skill.sourceUrl,
        skill.license,
        ...roles.flatMap((role) => [role.label, role.labelZh]),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [normalizedQuery, skills]);

  const headerActions = (
    <View style={styles.headerActions}>
      <Pressable style={styles.ghostAction} accessibilityRole="button">
        <RefreshCcw size={16} color={theme.colors.foregroundMuted} />
        <Text style={styles.ghostActionText}>{text.skillRefresh}</Text>
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
      <Pressable style={styles.primaryAction} accessibilityRole="button">
        <Plus size={18} color={theme.colors.palette.black} />
        <Text style={styles.primaryActionText}>{text.skillNew}</Text>
      </Pressable>
    </View>
  );

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
          <Text style={styles.subtitle}>{text.skillLibraryPageSubtitle}</Text>
        </View>

        <View style={styles.sectionHeader}>
          <Text style={styles.sectionTitle}>{text.skillInstalled}</Text>
          <Text style={styles.sectionCount}>{filteredSkills.length}</Text>
        </View>

        <View style={styles.skillGrid}>
          {filteredSkills.map(({ skill, roles }) => (
            <View key={skill.id} style={styles.skillRow}>
              <View style={styles.skillIcon}>
                <Box size={22} color={theme.colors.accentBright} />
              </View>
              <View style={styles.skillCopy}>
                <View style={styles.skillTitleRow}>
                  <Text style={styles.skillName} numberOfLines={1}>
                    {skill.name}
                  </Text>
                  <Text style={styles.skillMeta} numberOfLines={1}>
                    {roles.map((role) => (isZh ? role.labelZh : role.label)).join(", ")}
                  </Text>
                </View>
                <Text style={styles.skillDescription} numberOfLines={1}>
                  {skill.description}
                </Text>
                <Text style={styles.skillSource} numberOfLines={1}>
                  {skill.sourceUrl} · {skill.license}
                </Text>
              </View>
              <Check size={18} color={theme.colors.foregroundMuted} />
            </View>
          ))}
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
    borderRadius: theme.borderRadius.lg,
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
  primaryAction: {
    height: 40,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.foreground,
  },
  primaryActionText: {
    color: theme.colors.palette.black,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
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
    columnGap: theme.spacing[12],
    rowGap: theme.spacing[8],
  },
  skillRow: {
    width: "47%",
    minWidth: 360,
    minHeight: 72,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[4],
  },
  skillIcon: {
    width: 52,
    height: 52,
    borderRadius: theme.borderRadius.lg,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.surface1,
    flexShrink: 0,
  },
  skillCopy: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
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
  skillMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    flexShrink: 0,
  },
  skillDescription: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  skillSource: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
}));
