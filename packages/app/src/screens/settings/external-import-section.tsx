import { useCallback, useMemo, useState } from "react";
import { Alert, Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Check, Download, Square } from "lucide-react-native";

import { SettingsSection } from "@/screens/settings/settings-section";
import { StatusPanel } from "@/components/ui/status-panel";
import { Button } from "@/components/ui/button";
import { settingsStyles } from "@/styles/settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import {
  importExternalProviders,
  scanExternalProviders,
  shouldUseDesktopDaemon,
  type ExternalImportScan,
  type ExternalProviderCandidate,
} from "@/desktop/daemon/desktop-daemon";

interface ExternalImportSectionProps {
  /** Names of endpoints already saved, used to warn about overwrites before committing. */
  existingNames: string[];
  onImported?: () => void;
}

export function ExternalImportSection({ existingNames, onImported }: ExternalImportSectionProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.externalImport, [locale]);

  const [scans, setScans] = useState<ExternalImportScan[] | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [isImporting, setIsImporting] = useState(false);
  const [confirmingOverwrite, setConfirmingOverwrite] = useState(false);

  const existing = useMemo(
    () => new Set(existingNames.map((name) => name.trim().toLowerCase())),
    [existingNames],
  );

  const handleScan = useCallback(() => {
    setIsScanning(true);
    setConfirmingOverwrite(false);
    void scanExternalProviders()
      .then((result) => {
        setScans(result);
        setSelected(new Set());
      })
      .catch((error) => {
        console.error("[ExternalImport] Scan failed", error);
        setScans([]);
      })
      .finally(() => setIsScanning(false));
  }, []);

  const allItems = useMemo(() => (scans ?? []).flatMap((scan) => scan.items), [scans]);
  const selectedItems = useMemo(
    () => allItems.filter((item) => selected.has(item.id)),
    [allItems, selected],
  );
  const conflictCount = useMemo(
    () => selectedItems.filter((item) => existing.has(item.name.trim().toLowerCase())).length,
    [selectedItems, existing],
  );

  const toggle = useCallback((id: string) => {
    setConfirmingOverwrite(false);
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const handleImport = useCallback(() => {
    // Overwrites are confirmed inline: the first press relabels the button and explains
    // exactly what is preserved, the second commits.
    if (conflictCount > 0 && !confirmingOverwrite) {
      setConfirmingOverwrite(true);
      return;
    }
    setIsImporting(true);
    void importExternalProviders([...selected])
      .then((result) => {
        Alert.alert(text.title, text.imported(result.imported));
        setSelected(new Set());
        setConfirmingOverwrite(false);
        onImported?.();
        handleScan();
      })
      .catch((error) => {
        console.error("[ExternalImport] Import failed", error);
        Alert.alert(text.importFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => setIsImporting(false));
  }, [conflictCount, confirmingOverwrite, selected, text, onImported, handleScan]);

  if (!shouldUseDesktopDaemon()) {
    return null;
  }

  const trailing = (
    <Button variant="ghost" size="sm" busy={isScanning} onPress={handleScan}>
      {isScanning ? text.scanning : text.scan}
    </Button>
  );

  return (
    <SettingsSection title={text.title} trailing={trailing}>
      <Text style={styles.subtitle}>{text.subtitle}</Text>

      {scans === null ? null : allItems.length === 0 ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.nothingFound} description={text.nothingFoundHint} />
        </View>
      ) : (
        <>
          {scans.map((scan) => (
            <View key={scan.source} style={styles.sourceRow}>
              <View
                style={[styles.sourceDot, scan.detected && styles.sourceDotOnline]}
                accessibilityElementsHidden
              />
              <Text style={styles.sourceName}>{text.sources[scan.source]}</Text>
              <Text style={styles.sourcePath} numberOfLines={1}>
                {/* Showing the file we read is what makes the result verifiable. */}
                {scan.dataPath ? text.readFrom(scan.dataPath) : text.notDetected}
              </Text>
            </View>
          ))}

          <View style={settingsStyles.card}>
            {allItems.map((item, index) => (
              <ImportRow
                key={item.id}
                item={item}
                index={index}
                selected={selected.has(item.id)}
                willOverwrite={existing.has(item.name.trim().toLowerCase())}
                onToggle={toggle}
                text={text}
                accentColor={theme.colors.accent}
                mutedColor={theme.colors.foregroundMuted}
                warningColor={theme.colors.statusWarning}
              />
            ))}
          </View>

          {confirmingOverwrite ? (
            <Text style={styles.overwriteHint}>{text.overwriteHint(conflictCount)}</Text>
          ) : null}

          <View style={styles.footer}>
            <Text style={styles.selectedText}>{text.selected(selected.size)}</Text>
            {selected.size > 0 ? (
              <Button variant="ghost" size="sm" onPress={() => setSelected(new Set())}>
                {text.clear}
              </Button>
            ) : null}
            <Button
              variant="default"
              size="sm"
              leftIcon={Download}
              busy={isImporting}
              disabled={selected.size === 0}
              onPress={handleImport}
            >
              {confirmingOverwrite ? text.confirmOverwrite : text.import(selected.size)}
            </Button>
          </View>
        </>
      )}
    </SettingsSection>
  );
}

interface ImportRowProps {
  item: ExternalProviderCandidate;
  index: number;
  selected: boolean;
  willOverwrite: boolean;
  onToggle: (id: string) => void;
  text: ReturnType<typeof getSub2APIMessages>["settings"]["externalImport"];
  accentColor: string;
  mutedColor: string;
  warningColor: string;
}

function ImportRow({
  item,
  index,
  selected,
  willOverwrite,
  onToggle,
  text,
  accentColor,
  mutedColor,
  warningColor,
}: ImportRowProps) {
  const meta = [
    item.target === "claude" ? "Claude Code" : "Codex",
    item.models.length > 0 ? text.modelCount(item.models.length) : null,
    item.hasApiKey ? null : text.noApiKey,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <Pressable
      onPress={() => onToggle(item.id)}
      style={[settingsStyles.row, index > 0 && settingsStyles.rowBorder]}
      accessibilityRole="checkbox"
      accessibilityState={{ checked: selected }}
    >
      {selected ? <Check size={16} color={accentColor} /> : <Square size={16} color={mutedColor} />}
      <View style={settingsStyles.rowContent}>
        <Text style={settingsStyles.rowTitle} numberOfLines={1}>
          {item.name}
        </Text>
        <Text style={settingsStyles.rowHint} numberOfLines={1}>
          {meta}
        </Text>
        <Text style={settingsStyles.rowHint} numberOfLines={1}>
          {item.baseUrl}
        </Text>
      </View>
      {willOverwrite ? (
        <Text style={[styles.conflict, { color: warningColor }]}>{text.willOverwrite}</Text>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create((theme) => ({
  subtitle: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginBottom: theme.spacing[2],
  },
  sourceRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingVertical: 4,
  },
  sourceDot: {
    width: 8,
    height: 8,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.border,
  },
  sourceDotOnline: {
    backgroundColor: theme.colors.statusSuccess,
  },
  sourceName: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
  },
  sourcePath: {
    flex: 1,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  conflict: {
    fontSize: theme.fontSize.xs,
  },
  overwriteHint: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginTop: theme.spacing[2],
  },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: theme.spacing[2],
    marginTop: theme.spacing[2],
  },
  selectedText: {
    flex: 1,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
}));
