import { Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { PairingProvider } from '../lib/pairing';
import { useTheme } from '../lib/theme';

export default function Layout() {
  const t = useTheme();
  return (
    <SafeAreaProvider>
      <PairingProvider>
        <StatusBar style="auto" />
        <Stack
          screenOptions={{
            headerStyle: { backgroundColor: t.canvas },
            headerTintColor: t.text,
            headerShadowVisible: false,
            contentStyle: { backgroundColor: t.canvas },
          }}
        >
          <Stack.Screen name="index" options={{ title: 'Sessions' }} />
          <Stack.Screen name="pair" options={{ title: 'Pair with Runo', presentation: 'modal' }} />
          <Stack.Screen name="session/[id]" options={{ title: '' }} />
        </Stack>
      </PairingProvider>
    </SafeAreaProvider>
  );
}
