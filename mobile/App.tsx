import { StatusBar } from 'expo-status-bar';
import { useState } from 'react';
import { Pressable, SafeAreaView, ScrollView, StyleSheet, Text, View } from 'react-native';

// A sample screen to check Expo runs on the phone. It uses demo data only.

type Status = 'working' | 'needs' | 'done';

type Session = {
  id: number;
  title: string;
  agent: string;
  branch: string;
  status: Status;
  preview: string;
};

const SESSIONS: Session[] = [
  {
    id: 1,
    title: 'Fix tab overflow',
    agent: 'Claude Code',
    branch: 'fix/tab-overflow',
    status: 'needs',
    preview: 'Wants to run `cargo test`',
  },
  {
    id: 2,
    title: 'Mission board polish',
    agent: 'Codex',
    branch: 'feat/mission-board',
    status: 'working',
    preview: 'Looking at mission_view.rs…',
  },
  {
    id: 3,
    title: 'Proof of done',
    agent: 'Claude Code',
    branch: 'feat/proof-of-done',
    status: 'done',
    preview: 'Checks passed. Ready to merge.',
  },
];

const C = {
  canvas: '#18181b',
  raised: '#1f1f23',
  border: '#2a2a2f',
  text: '#e9e9ec',
  muted: '#93939d',
  accent: '#5c7cfa',
  green: '#4ade80',
  amber: '#f3b23f',
};

const LOOK: Record<Status, { color: string; label: string }> = {
  working: { color: C.accent, label: 'Working' },
  needs: { color: C.amber, label: 'Needs you' },
  done: { color: C.green, label: 'Done' },
};

export default function App() {
  const [sessions, setSessions] = useState(SESSIONS);
  const waiting = sessions.filter((s) => s.status === 'needs').length;

  const allow = (id: number) =>
    setSessions((list) =>
      list.map((s) =>
        s.id === id ? { ...s, status: 'working', preview: 'Running `cargo test`…' } : s,
      ),
    );

  return (
    <SafeAreaView style={styles.screen}>
      <StatusBar style="light" />
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.brand}>Runo</Text>
        <Text style={styles.subtitle}>
          {waiting === 0 ? 'Nothing needs you' : `${waiting} session needs you`}
        </Text>

        {sessions.map((s) => {
          const look = LOOK[s.status];
          return (
            <View key={s.id} style={styles.card}>
              <View style={styles.row}>
                <Text style={styles.title} numberOfLines={1}>
                  {s.title}
                </Text>
                <View style={[styles.chip, { borderColor: look.color }]}>
                  <View style={[styles.dot, { backgroundColor: look.color }]} />
                  <Text style={[styles.chipText, { color: look.color }]}>{look.label}</Text>
                </View>
              </View>
              <Text style={styles.meta}>
                {s.agent} · {s.branch}
              </Text>
              <Text style={styles.preview}>{s.preview}</Text>
              {s.status === 'needs' && (
                <Pressable
                  onPress={() => allow(s.id)}
                  style={({ pressed }) => [styles.button, pressed && { opacity: 0.7 }]}
                >
                  <Text style={styles.buttonText}>Allow</Text>
                </Pressable>
              )}
            </View>
          );
        })}

        <Text style={styles.footer}>Sample app · demo data</Text>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: C.canvas },
  content: { padding: 20, paddingTop: 48, gap: 12 },
  brand: { color: C.text, fontSize: 28, fontWeight: '700' },
  subtitle: { color: C.muted, fontSize: 14, marginBottom: 8 },
  card: {
    backgroundColor: C.raised,
    borderColor: C.border,
    borderWidth: 1,
    borderRadius: 14,
    padding: 16,
    gap: 6,
  },
  row: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  title: { flex: 1, color: C.text, fontSize: 16, fontWeight: '600' },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 3,
  },
  dot: { width: 6, height: 6, borderRadius: 3 },
  chipText: { fontSize: 12, fontWeight: '600' },
  meta: { color: C.muted, fontSize: 12 },
  preview: { color: C.text, fontSize: 14, opacity: 0.85 },
  button: {
    marginTop: 6,
    alignSelf: 'flex-start',
    backgroundColor: C.accent,
    borderRadius: 10,
    paddingHorizontal: 18,
    paddingVertical: 9,
  },
  buttonText: { color: '#fff', fontWeight: '600' },
  footer: { color: C.muted, fontSize: 12, textAlign: 'center', marginTop: 16 },
});
