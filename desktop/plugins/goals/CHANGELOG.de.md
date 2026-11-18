# Changes

## 0.2.3

- Testnamen kürzen, ohne die Prüfungen zu ändern
- Verlaufstestdaten des Produktions-SDK korrigieren und konfigurierte Ziellimits prüfen

## 0.2.2

- Das Statusabzeichen zeigt nach dem Symbol den Zustand und die Zeit, nach einer Pause den Grund; das Wort Ziel wird nicht mehr wiederholt
- Das Panel hat Pausieren oder Fortsetzen und Beenden. Bearbeiten entfällt: `/goal <neuer Text>` ersetzt das Ziel. Löschen heißt jetzt Beenden, `/goal end`, und ein beendetes Ziel steht im Verlauf als Beendet
- Ein fehlgeschlagener Ziel-Turn wird nur einmal gemeldet, durch die Benachrichtigung der App; das Ziel zeigt weiterhin Pausiert · Fehler

## 0.2.1

- Ein Ziel, das nichts mehr wecken kann, bleibt nicht mehr auf „Wartet“ oder „Läuft“: Es wird pausiert angezeigt, nachdem die erwartete Hintergrundarbeit gestoppt oder ihr Ergebnis am Limit zurückgehalten wurde oder der letzte Turn vor einem Neustart endete

## 0.2.0

- `/goal <Text>` startet sofort und läuft in automatischen Runden weiter, bis das Ziel erreicht ist; eine Nachricht oder Pause bleibt Teil des Ziels
- Der Abschluss ist eine Meldung mit Belegen, die eine eigene Runde „Ziel prüfen“ bestätigen muss; Wieder öffnen macht ihn rückgängig
- Stopp pausiert und eine Antwort setzt fort; Pausieren hält bis Fortsetzen; Neustart, Abzweigung, Zurückspulen oder Planmodus pausieren mit ihrem Grund
- Grenzen: standardmäßig 25 automatische Runden, optionale Token- und Zeitbudgets sowie eine Regel für fehlenden Fortschritt; eine erreichte Grenze schließt ab und pausiert
- Drei Werkzeuge ersetzen sechs; nur Sie bearbeiten, ersetzen oder löschen das Ziel

## 0.1.10

- Umrisssymbole in Navigation und Werkzeugen wiederhergestellt

## 0.1.9

- Neues Plugin-Symbol

## 0.1.6

- Inaktive Ziele ohne Abfragen belassen und nur Statusänderungen verfolgen

## 0.1.4

- Marktkategorie und Symbolfarbe hinzugefügt
- Übersetzte Werkzeugnamen hinzugefügt
