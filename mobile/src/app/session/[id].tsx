import { Stack, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Platform,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Banner, Button, Card, Dot, statusColor } from '../../components/ui';
import { segments, statusLabel } from '../../lib/format';
import { usePairing } from '../../lib/pairing';
import { mono, useTheme } from '../../lib/theme';
import type { Approval, Message, SessionDetail } from '../../lib/types';
import { usePoll } from '../../lib/usePoll';

export default function Session() {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const id = Number(useLocalSearchParams<{ id: string }>().id);
  const { bridge } = usePairing();
  const load = useMemo(() => (bridge ? () => bridge.session(id) : null), [bridge, id]);
  const { data, error, refresh } = usePoll(load, 2000);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const list = useRef<FlatList<Message>>(null);

  /** Run an action, then refresh; the desktop app picks it up within a couple of seconds. */
  const act = useCallback(
    async (key: string, run: () => Promise<unknown>) => {
      setBusy(key);
      setActionError(null);
      try {
        await run();
        setTimeout(() => void refresh(), 1500);
        return true;
      } catch (e) {
        setActionError(e instanceof Error ? e.message : String(e));
        return false;
      } finally {
        setBusy(null);
      }
    },
    [refresh],
  );

  const count = data?.messages.length ?? 0;
  useEffect(() => {
    if (count) setTimeout(() => list.current?.scrollToEnd({ animated: false }), 50);
  }, [count]);

  if (!bridge) return <Banner text="Pair with a computer first." />;

  const s = data?.session;
  const working = s?.status === 'working';
  const last = data?.messages.length ? data.messages.length - 1 : -1;

  const send = async () => {
    const text = draft.trim();
    if (!text) return;
    if (await act('send', () => bridge.prompt(id, text))) setDraft('');
  };

  const stop = () =>
    Alert.alert('Stop the agent?', 'It stops mid-turn, the same as Stop in Runo.', [
      { text: 'Keep going', style: 'cancel' },
      { text: 'Stop', style: 'destructive', onPress: () => void act('stop', () => bridge.stop(id)) },
    ]);

  return (
    <KeyboardAvoidingView
      style={{ flex: 1 }}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      keyboardVerticalOffset={Platform.OS === 'ios' ? 90 : 0}
    >
      <Stack.Screen
        options={{
          title: s?.title ?? '',
          headerRight: () =>
            working ? <Button label="Stop" danger busy={busy === 'stop'} onPress={stop} style={{ minHeight: 32 }} /> : null,
        }}
      />
      {s ? (
        <View style={[styles.status, { borderColor: t.border }]}>
          <Dot color={statusColor(t, s.status, s.needs)} />
          <Text style={{ color: t.secondary, fontSize: 13 }}>
            {[s.agent, statusLabel(s.status), s.branch, s.elsewhere ? 'on a connection' : '']
              .filter(Boolean)
              .join(' · ')}
          </Text>
        </View>
      ) : null}
      {error ? <Banner text={error} /> : null}
      <FlatList
        ref={list}
        data={data?.messages ?? []}
        keyExtractor={(_, i) => String(i)}
        contentContainerStyle={{ padding: 12, gap: 10 }}
        renderItem={({ item, index }) => (
          <Bubble
            message={item}
            last={index === last && !working}
            busy={busy}
            onPlan={() => act('plan', () => bridge.approvePlan(id))}
            onAllow={(tools) => act('allow', () => bridge.allow(id, tools))}
            onAnswer={(picks) => act('answer', () => bridge.answer(id, picks))}
          />
        )}
        ListFooterComponent={data ? <Footer detail={data} busy={busy} act={act} /> : null}
      />
      {actionError ? <Banner text={actionError} /> : null}
      <View style={[styles.composer, { borderColor: t.border, paddingBottom: 8 + insets.bottom }]}>
        <TextInput
          style={[styles.input, { color: t.text, backgroundColor: t.surface, borderColor: t.border }]}
          value={draft}
          onChangeText={setDraft}
          placeholder={working ? 'Queue a follow-up…' : 'Reply to the agent…'}
          placeholderTextColor={t.muted}
          multiline
        />
        <Button label="Send" primary busy={busy === 'send'} disabled={!draft.trim()} onPress={send} />
      </View>
    </KeyboardAvoidingView>
  );
}

function Bubble({
  message: m,
  last,
  busy,
  onPlan,
  onAllow,
  onAnswer,
}: {
  message: Message;
  last: boolean;
  busy: string | null;
  onPlan: () => void;
  onAllow: (tools: string[]) => void;
  onAnswer: (picks: (string | null)[]) => void;
}) {
  const t = useTheme();
  const [picks, setPicks] = useState<(string | null)[]>([]);
  if (m.role === 'system') {
    return <Text style={[styles.system, { color: t.muted }]}>{m.text}</Text>;
  }
  const user = m.role === 'user';
  const text = m.playbook ? `▶ ${m.playbook}${m.text ? `\n${m.text}` : ''}` : m.text;
  const asked = m.questions.filter((q) => q.options.length);
  return (
    <View style={{ alignItems: user ? 'flex-end' : 'stretch', gap: 6 }}>
      {m.steps.length ? (
        <Text style={{ color: t.muted, fontSize: 12 }}>{m.steps.join(' · ')}</Text>
      ) : null}
      {text ? (
        <View style={[styles.bubble, { backgroundColor: user ? t.raised : 'transparent' }, user && { maxWidth: '85%' }]}>
          {segments(text).map((seg, i) =>
            seg.code ? (
              <Text key={i} selectable style={[styles.code, { backgroundColor: t.code, color: t.text }]}>
                {seg.text}
              </Text>
            ) : (
              <Text key={i} selectable style={{ color: t.text, fontSize: 15, lineHeight: 21 }}>
                {seg.text}
              </Text>
            ),
          )}
        </View>
      ) : null}
      {m.plan ? (
        <Card tint={last ? t.amber : undefined}>
          <Text style={[styles.cardTitle, { color: t.text }]}>Plan</Text>
          <Text selectable style={{ color: t.secondary, lineHeight: 20 }}>
            {m.plan}
          </Text>
          {last ? <Button label="Approve and build" primary busy={busy === 'plan'} onPress={onPlan} /> : null}
        </Card>
      ) : null}
      {last && asked.length ? (
        <Card tint={t.amber}>
          {asked.map((q, qi) => (
            <View key={qi} style={{ gap: 6 }}>
              <Text style={[styles.cardTitle, { color: t.text }]}>{q.question}</Text>
              <View style={styles.options}>
                {q.options.map((o) => (
                  <Button
                    key={o}
                    label={o}
                    primary={picks[qi] === o}
                    onPress={() => {
                      const next = asked.map((_, i) => picks[i] ?? null);
                      next[qi] = o;
                      setPicks(next);
                    }}
                  />
                ))}
              </View>
            </View>
          ))}
          <Button
            label="Send answers"
            primary
            busy={busy === 'answer'}
            disabled={!picks.some(Boolean)}
            onPress={() => onAnswer(asked.map((_, i) => picks[i] ?? null))}
          />
        </Card>
      ) : null}
      {last && m.denied.length ? (
        <Card tint={t.amber}>
          <Text style={{ color: t.text }}>
            Wants to use {m.denied.join(', ')}, which wasn’t allowed.
          </Text>
          <Button label="Allow once and continue" primary busy={busy === 'allow'} onPress={() => onAllow(m.denied)} />
        </Card>
      ) : null}
      {m.meta ? <Text style={{ color: t.muted, fontSize: 12 }}>{m.meta}</Text> : null}
    </View>
  );
}

function Footer({
  detail,
  busy,
  act,
}: {
  detail: SessionDetail;
  busy: string | null;
  act: (key: string, run: () => Promise<unknown>) => Promise<boolean>;
}) {
  const t = useTheme();
  const { bridge } = usePairing();
  if (!bridge) return null;
  return (
    <View style={{ gap: 10, marginTop: 6 }}>
      {detail.approvals.map((a: Approval) => (
        <Card key={a.id} tint={t.amber}>
          <Text style={[styles.cardTitle, { color: t.text }]}>Permission needed</Text>
          <Text style={{ color: t.secondary }}>
            {detail.session.agent} wants to {a.what}.
          </Text>
          <View style={styles.options}>
            <Button
              label="Allow"
              primary
              busy={busy === `ok:${a.id}`}
              onPress={() => act(`ok:${a.id}`, () => bridge.approval(a.id, true))}
            />
            <Button
              label="Deny"
              danger
              busy={busy === `no:${a.id}`}
              onPress={() => act(`no:${a.id}`, () => bridge.approval(a.id, false))}
            />
          </View>
        </Card>
      ))}
      {detail.queue.map((q, i) => (
        <Text key={i} style={[styles.system, { color: t.muted }]}>
          Queued: {q}
        </Text>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  status: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    paddingHorizontal: 16,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  bubble: { borderRadius: 14, paddingHorizontal: 12, paddingVertical: 8, gap: 8 },
  code: { fontFamily: mono, fontSize: 13, padding: 10, borderRadius: 8 },
  system: { fontSize: 13, textAlign: 'center' },
  cardTitle: { fontSize: 15, fontWeight: '600' },
  options: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  composer: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    gap: 8,
    paddingHorizontal: 12,
    paddingTop: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  input: {
    flex: 1,
    maxHeight: 140,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingTop: 10,
    paddingBottom: 10,
    fontSize: 15,
  },
});
