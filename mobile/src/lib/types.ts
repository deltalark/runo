// The phone bridge's JSON, as served by `runo --phone` (native/runo-app/src/phone.rs).

export type Status = 'idle' | 'working' | 'done' | 'failed';

export type NeedKind = 'approval' | 'denied' | 'questions' | 'plan';

export interface Need {
  kind: NeedKind;
  text: string;
}

export interface Project {
  id: number;
  name: string;
  color: string;
}

export interface SessionSummary {
  id: number;
  project: number;
  title: string;
  agent: string;
  branch: string;
  status: Status;
  unread: boolean;
  pinned: boolean;
  archived: boolean;
  /** Runs on a connection (SSH, Cove) instead of the desktop itself. */
  elsewhere: boolean;
  updated: string;
  preview: string;
  needs: Need | null;
}

export interface SessionList {
  projects: Project[];
  sessions: SessionSummary[];
}

export interface Question {
  question: string;
  options: string[];
}

export interface Message {
  role: 'user' | 'agent' | 'system';
  text: string;
  playbook: string;
  steps: string[];
  time: string;
  meta: string;
  plan: string;
  questions: Question[];
  denied: string[];
}

export interface Approval {
  id: string;
  tool: string;
  what: string;
  asked: number;
}

export interface SessionDetail {
  session: SessionSummary;
  messages: Message[];
  queue: string[];
  approvals: Approval[];
}

export interface Hello {
  app: 'runo';
  version: string;
  api: number;
}
