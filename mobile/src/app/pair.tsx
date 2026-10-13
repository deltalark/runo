import { CameraView, useCameraPermissions } from 'expo-camera';
import * as Clipboard from 'expo-clipboard';
import { router, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { Banner, Button } from '../components/ui';
import { Bridge } from '../lib/api';
import { normalizeUrl, parsePairLink, type Pairing } from '../lib/pair';
import { usePairing } from '../lib/pairing';
import { mono, useTheme } from '../lib/theme';

/**
 * Pair with a computer running `runo --phone`: scan or paste the link it prints, or type
 * the address and token. Opening a runo://pair link lands here with both filled in.
 */
export default function Pair() {
  const t = useTheme();
  const params = useLocalSearchParams<{ url?: string; token?: string }>();
  const { bridge, save, forget } = usePairing();
  const [url, setUrl] = useState(params.url ?? bridge?.pairing.url ?? '');
  const [token, setToken] = useState(params.token ?? '');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [permission, requestPermission] = useCameraPermissions();
  const scanned = useRef(false);

  const connect = useCallback(
    async (pairing: Pairing) => {
      setBusy(true);
      setError(null);
      try {
        const hello = await new Bridge(pairing).hello();
        if (hello.app !== 'runo') throw new Error('That address isn’t Runo’s phone bridge.');
        await save(pairing);
        router.dismissTo('/');
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [save],
  );

  // A pairing link that opened the app connects straight away.
  useEffect(() => {
    if (params.url && params.token) {
      void connect({ url: normalizeUrl(params.url), token: params.token });
    }
  }, [params.url, params.token, connect]);

  const usePasted = async () => {
    const pairing = parsePairLink(await Clipboard.getStringAsync());
    if (!pairing) {
      setError('The clipboard doesn’t hold a runo://pair link. Copy the one `runo --phone` printed.');
      return;
    }
    setUrl(pairing.url);
    setToken(pairing.token);
    await connect(pairing);
  };

  const startScan = async () => {
    if (!permission?.granted && !(await requestPermission()).granted) {
      setError('Runo needs the camera to scan the pairing code. You can paste the link instead.');
      return;
    }
    scanned.current = false;
    setScanning(true);
  };

  const input = [styles.input, { color: t.text, backgroundColor: t.surface, borderColor: t.border }];

  if (scanning) {
    return (
      <View style={{ flex: 1 }}>
        <CameraView
          style={{ flex: 1 }}
          barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
          onBarcodeScanned={({ data }) => {
            const pairing = parsePairLink(data);
            if (!pairing || scanned.current) return;
            scanned.current = true;
            setScanning(false);
            setUrl(pairing.url);
            setToken(pairing.token);
            void connect(pairing);
          }}
        />
        <Button label="Cancel" onPress={() => setScanning(false)} style={{ margin: 16 }} />
      </View>
    );
  }

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        <Text style={[styles.lead, { color: t.secondary }]}>
          On your computer, run <Text style={{ fontFamily: mono, color: t.text }}>runo --phone</Text>. It prints a
          pairing link. Scan it as a QR code, or copy it to this phone and paste it here.
        </Text>
        <View style={styles.actions}>
          <Button label="Scan code" primary onPress={startScan} style={{ flex: 1 }} />
          <Button label="Paste link" onPress={usePasted} style={{ flex: 1 }} />
        </View>
        {error ? <Banner text={error} /> : null}
        <Text style={[styles.label, { color: t.muted }]}>OR TYPE IT IN</Text>
        <TextInput
          style={input}
          value={url}
          onChangeText={setUrl}
          placeholder="http://100.64.0.2:7631"
          placeholderTextColor={t.muted}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
        />
        <TextInput
          style={[...input, { fontFamily: mono }]}
          value={token}
          onChangeText={setToken}
          placeholder="Token"
          placeholderTextColor={t.muted}
          autoCapitalize="none"
          autoCorrect={false}
          secureTextEntry
        />
        <Button
          label="Connect"
          primary
          busy={busy}
          disabled={!url.trim() || !token.trim()}
          onPress={() => connect({ url: normalizeUrl(url), token: token.trim() })}
        />
        <Text style={[styles.note, { color: t.muted }]}>
          Away from home, put the computer and this phone on the same Tailscale network and use the
          computer’s Tailscale address. Don’t expose the bridge to the open internet.
        </Text>
        {bridge ? (
          <Button
            label={`Forget ${bridge.pairing.url}`}
            danger
            onPress={async () => {
              await forget();
              setToken('');
            }}
          />
        ) : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  body: { padding: 16, gap: 12 },
  lead: { fontSize: 15, lineHeight: 21 },
  actions: { flexDirection: 'row', gap: 10 },
  label: { fontSize: 12, fontWeight: '600', letterSpacing: 0.6, marginTop: 12 },
  input: { borderWidth: 1, borderRadius: 10, paddingHorizontal: 12, paddingVertical: 10, fontSize: 15 },
  note: { fontSize: 13, lineHeight: 18 },
});
