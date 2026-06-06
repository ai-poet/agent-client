import Svg, { Path, Circle, G } from "react-native-svg";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import type { GraphSvgLayerProps } from "./graph-types";

export function GraphSvgLayer({ layout, selectedHash, headHash }: GraphSvgLayerProps) {
  const { theme } = useUnistyles();

  return (
    <Svg width={layout.width} height={layout.height} style={styles.svgLayer}>
      {/* Edges: render lines first so nodes appear on top */}
      {layout.edges.map((edge) => (
        <Path
          key={`${edge.from}-${edge.to}`}
          d={edge.path}
          stroke={edge.color}
          strokeWidth={edge.isMerge ? 1.5 : 2}
          strokeDasharray={edge.isMerge ? "3,2" : undefined}
          fill="none"
          opacity={0.85}
        />
      ))}

      {/* Nodes */}
      {layout.nodes.map((node) => {
        const isSelected = selectedHash === node.commit.fullHash;
        const isHead = headHash === node.commit.fullHash;
        const radius = isSelected ? layout.nodeRadius + 1 : layout.nodeRadius;
        const strokeWidth = isSelected ? 2.5 : 1.5;

        return (
          <G key={node.commit.fullHash}>
            {/* HEAD outer ring */}
            {isHead && (
              <Circle
                cx={node.x}
                cy={node.y}
                r={radius + 4}
                fill="none"
                stroke={theme.colors.accent}
                strokeWidth={2}
                opacity={0.6}
              />
            )}
            {/* Commit node */}
            <Circle
              cx={node.x}
              cy={node.y}
              r={radius}
              fill={isSelected ? node.color : theme.colors.background}
              stroke={node.color}
              strokeWidth={strokeWidth}
            />
          </G>
        );
      })}
    </Svg>
  );
}

const styles = StyleSheet.create(() => ({
  svgLayer: {
    position: "absolute",
    left: 0,
    top: 0,
    zIndex: 1,
  },
}));
