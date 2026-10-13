import { Platform, useColorScheme } from 'react-native';

const light = {
  canvas: '#f6f6f4',
  surface: '#ffffff',
  raised: '#eeeeeb',
  border: '#e2e2de',
  text: '#1b1b19',
  secondary: '#5f5f5a',
  muted: '#8e8e88',
  accent: '#2f6df6',
  onAccent: '#ffffff',
  green: '#1f9d55',
  amber: '#c27c0e',
  red: '#d64545',
  code: '#f0f0ed',
};

type Palette = typeof light;

const dark: Palette = {
  canvas: '#121211',
  surface: '#1b1b1a',
  raised: '#252523',
  border: '#2e2e2c',
  text: '#ececea',
  secondary: '#a9a9a3',
  muted: '#76766f',
  accent: '#5b8dff',
  onAccent: '#0b0b0b',
  green: '#3ecf7a',
  amber: '#f0a83a',
  red: '#f06a6a',
  code: '#222220',
};

export type Theme = Palette;

export function useTheme(): Theme {
  return useColorScheme() === 'dark' ? dark : light;
}

export const mono = Platform.select({ ios: 'Menlo', default: 'monospace' });
