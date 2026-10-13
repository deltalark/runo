import * as SecureStore from 'expo-secure-store';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Bridge } from './api';
import type { Pairing } from './pair';

const KEY = 'runo.pairing';

interface PairingState {
  /** False until the saved pairing has been read. */
  ready: boolean;
  bridge: Bridge | null;
  save: (pairing: Pairing) => Promise<void>;
  forget: () => Promise<void>;
}

const Context = createContext<PairingState | null>(null);

/** The paired computer, kept in the keychain (iOS) or Keystore (Android). */
export function PairingProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false);
  const [pairing, setPairing] = useState<Pairing | null>(null);

  useEffect(() => {
    SecureStore.getItemAsync(KEY)
      .then((saved) => {
        if (saved) setPairing(JSON.parse(saved) as Pairing);
      })
      .catch(() => {})
      .finally(() => setReady(true));
  }, []);

  const save = useCallback(async (next: Pairing) => {
    await SecureStore.setItemAsync(KEY, JSON.stringify(next));
    setPairing(next);
  }, []);

  const forget = useCallback(async () => {
    await SecureStore.deleteItemAsync(KEY);
    setPairing(null);
  }, []);

  const bridge = useMemo(() => (pairing ? new Bridge(pairing) : null), [pairing]);
  const value = useMemo(() => ({ ready, bridge, save, forget }), [ready, bridge, save, forget]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function usePairing(): PairingState {
  const value = useContext(Context);
  if (!value) throw new Error('usePairing needs PairingProvider');
  return value;
}
