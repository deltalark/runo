import type { ReactNode } from 'react';
import { ActivityIndicator, Pressable, StyleSheet, Text, View, type ViewStyle } from 'react-native';
import type { Need, Status } from '../lib/types';
import { useTheme, type Theme } from '../lib/theme';

export function statusColor(t: Theme, status: Status, needs: Need | null): string {
  if (needs) return t.amber;
  switch (status) {
    case 'working':
      return t.accent;
    case 'failed':
      return t.red;
    case 'done':
      return t.green;
    default:
      return t.muted;
  }
}

export function Dot({ color }: { color: string }) {
  return <View style={[styles.dot, { backgroundColor: color }]} />;
}

export function Button({
  label,
  onPress,
  primary,
  danger,
  busy,
  disabled,
  style,
}: {
  label: string;
  onPress: () => void;
  primary?: boolean;
  danger?: boolean;
  busy?: boolean;
  disabled?: boolean;
  style?: ViewStyle;
}) {
  const t = useTheme();
  const bg = primary ? t.accent : t.raised;
  const fg = primary ? t.onAccent : danger ? t.red : t.text;
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled || busy}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: bg, opacity: disabled ? 0.4 : pressed ? 0.7 : 1 },
        style,
      ]}
    >
      {busy ? <ActivityIndicator color={fg} /> : <Text style={[styles.buttonText, { color: fg }]}>{label}</Text>}
    </Pressable>
  );
}

export function Card({ children, tint }: { children: ReactNode; tint?: string }) {
  const t = useTheme();
  return (
    <View style={[styles.card, { backgroundColor: t.surface, borderColor: tint ?? t.border }]}>{children}</View>
  );
}

export function Banner({ text }: { text: string }) {
  const t = useTheme();
  return (
    <View style={[styles.banner, { backgroundColor: t.raised }]}>
      <Text style={{ color: t.secondary }}>{text}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  dot: { width: 8, height: 8, borderRadius: 4 },
  button: {
    minHeight: 40,
    paddingHorizontal: 14,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonText: { fontSize: 15, fontWeight: '600' },
  card: { borderWidth: 1, borderRadius: 12, padding: 12, gap: 8 },
  banner: { margin: 12, padding: 12, borderRadius: 10 },
});
