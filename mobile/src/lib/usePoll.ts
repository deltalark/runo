import { useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { AppState } from 'react-native';

/**
 * Load `load()` now and every `everyMs` while the screen is focused and the app is in the
 * foreground. The bridge answers unchanged data with 304, so polling stays cheap.
 */
export function usePoll<T>(load: (() => Promise<T>) | null, everyMs: number) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const loadRef = useRef(load);
  loadRef.current = load;

  const refresh = useCallback(async () => {
    const run = loadRef.current;
    if (!run) return;
    try {
      setData(await run());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // Load at once when the loader arrives, such as right after the pairing is read.
  useEffect(() => {
    void refresh();
  }, [load, refresh]);

  useFocusEffect(
    useCallback(() => {
      let timer: ReturnType<typeof setInterval> | null = null;
      const start = () => {
        if (timer) return;
        void refresh();
        timer = setInterval(() => void refresh(), everyMs);
      };
      const stop = () => {
        if (timer) clearInterval(timer);
        timer = null;
      };
      start();
      const sub = AppState.addEventListener('change', (state) => (state === 'active' ? start() : stop()));
      return () => {
        stop();
        sub.remove();
      };
    }, [refresh, everyMs]),
  );

  return { data, error, refresh };
}
