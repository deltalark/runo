// Pure helpers for the screens. No React Native imports, so `node --test` can run them.

import type { Project, SessionSummary, Status } from './types';

export interface Section {
  key: string;
  title: string;
  data: SessionSummary[];
}

/**
 * The home list: what needs the person first, then what's running, then each project's
 * other sessions (pinned first). Archived sessions are left out.
 */
export function sections(projects: Project[], sessions: SessionSummary[]): Section[] {
  const live = sessions.filter((s) => !s.archived);
  const needs = live.filter((s) => s.needs);
  const working = live.filter((s) => !s.needs && s.status === 'working');
  const shown = new Set([...needs, ...working].map((s) => s.id));
  const out: Section[] = [];
  if (needs.length) out.push({ key: 'needs', title: 'Needs you', data: needs });
  if (working.length) out.push({ key: 'working', title: 'Working', data: working });
  const byProject = new Map<number, SessionSummary[]>();
  for (const s of live) {
    if (shown.has(s.id)) continue;
    const list = byProject.get(s.project) ?? [];
    list.push(s);
    byProject.set(s.project, list);
  }
  for (const p of projects) {
    const list = byProject.get(p.id);
    if (!list?.length) continue;
    // Newest first: session ids grow as they are made.
    list.sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.id - a.id);
    out.push({ key: `p${p.id}`, title: p.name, data: list });
  }
  return out;
}

export function statusLabel(status: Status): string {
  switch (status) {
    case 'working':
      return 'Working';
    case 'done':
      return 'Done';
    case 'failed':
      return 'Failed';
    default:
      return 'Idle';
  }
}

export type Segment = { code: false; text: string } | { code: true; text: string; lang: string };

/** Split a reply into prose and fenced code blocks, so code can be shown in monospace. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  const fence = /```([^\n`]*)\n([\s\S]*?)(?:```|$)/g;
  let last = 0;
  for (let m = fence.exec(text); m; m = fence.exec(text)) {
    const before = text.slice(last, m.index).trim();
    if (before) out.push({ code: false, text: before });
    out.push({ code: true, lang: m[1].trim(), text: m[2].replace(/\n$/, '') });
    last = fence.lastIndex;
  }
  const rest = text.slice(last).trim();
  if (rest) out.push({ code: false, text: rest });
  return out;
}
