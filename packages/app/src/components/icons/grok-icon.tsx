import Svg, { Path } from "react-native-svg";

interface GrokIconProps {
  size?: number;
  color?: string;
}

export function GrokIcon({ size = 16, color = "currentColor" }: GrokIconProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M6.469 8.776 16.512 24h4.759L11.228 8.776H6.469ZM6.47 15.855 2.729 24h4.784l1.348-2.936-2.392-5.209Z" />
      <Path d="M12.752 0 6.32 14.001l2.392 5.209L17.535 0h-4.783Z" />
      <Path d="M18.588 0 15.4 6.943 17.792 12.152 23.375 0h-4.787Z" />
    </Svg>
  );
}
