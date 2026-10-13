import { Link, Redirect, Stack, router } from 'expo-router';
import { useCallback, useMemo, useState } from 'react';
import { Pressable, RefreshControl, SectionList, StyleSheet, Text, View } from 'react-native';
import { Banner, Dot, statusColor } from '../components/ui';
import { sections, statusLabel } from '../lib/format';
import { usePairing } from '../lib/pairing';
import { useTheme } from '../lib/theme';
import type { SessionSummary } from '../lib/types';
import { usePoll } from '../lib/usePoll';

export default function Sessions() {
  const t = useTheme();
  const { ready, bridge } = usePairing();
  const load = useMemo(() => (bridge ? () => bridge.sessions() : null), [bridge]);
  const { data, error, refresh } = usePoll(load, 3000);
  const [pulling, setPulling] = useState(false);

  const onPull = useCallback(async () => {
    setPulling(true);
    await refresh();
    setPulling(false);
  }, [refresh]);

  const list = useMemo(() => (data ? sections(data.projects, data.sessions) : []), [data]);

  if (!ready) return null;
  if (!bridge) return <Redirect href="/pair" />;

  return (
    <>
      <Stack.Screen
        options={{
          headerRight: () => (
            <Link href="/pair" style={{ color: t.accent, fontSize: 16 }}>
              Pairing
            </Link>
          ),
        }}
      />
      <SectionList
        sections={list}
        keyExtractor={(s) => String(s.id)}
        stickySectionHeadersEnabled={false}
        refreshControl={<RefreshControl refreshing={pulling} onRefresh={onPull} />}
        ListHeaderComponent={error ? <Banner text={error} /> : null}
        ListEmptyComponent={
          data ? <Banner text="No sessions yet. Start one in Runo on your computer." /> : null
        }
        renderSectionHeader={({ section }) => (
          <Text style={[styles.header, { color: t.muted }]}>{section.title.toUpperCase()}</Text>
        )}
        renderItem={({ item }) => <Row session={item} />}
        contentContainerStyle={{ paddingBottom: 32 }}
      />
    </>
  );
}

function Row({ session: s }: { session: SessionSummary }) {
  const t = useTheme();
  const detail = s.needs?.text || s.preview || statusLabel(s.status);
  return (
    <Pressable
      onPress={() => router.push({ pathname: '/session/[id]', params: { id: String(s.id) } })}
      style={({ pressed }) => [styles.row, { backgroundColor: pressed ? t.raised : t.surface, borderColor: t.border }]}
    >
      <View style={styles.rowTop}>
        <Dot color={statusColor(t, s.status, s.needs)} />
        <Text numberOfLines={1} style={[styles.title, { color: t.text, fontWeight: s.unread ? '700' : '500' }]}>
          {s.title}
        </Text>
        <Text style={[styles.meta, { color: t.muted }]}>{s.agent}</Text>
      </View>
      <Text numberOfLines={2} style={{ color: s.needs ? t.amber : t.secondary, marginLeft: 16 }}>
        {detail}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  header: { fontSize: 12, fontWeight: '600', letterSpacing: 0.6, marginTop: 20, marginBottom: 6, marginHorizontal: 16 },
  row: {
    marginHorizontal: 12,
    marginVertical: 3,
    padding: 12,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    gap: 4,
  },
  rowTop: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  title: { flex: 1, fontSize: 16 },
  meta: { fontSize: 12 },
});
