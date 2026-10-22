export interface DesktopChoice {
  mode: string;
  label: string;
  available: boolean;
  reason?: string;
}

export interface DesktopOutput {
  name: string;
  width: number;
  height: number;
  scale?: number;
  x?: number;
  y?: number;
  description?: string;
}

export interface ControlState {
  ready: boolean;
  wanted: boolean;
  audioAvailable: boolean;
  audioEnabled: boolean;
  resizeAvailable: boolean;
  desktopMode: string;
  switchingDesktop: boolean;
  refreshingDesktops: boolean;
  choices: DesktopChoice[];
  outputs: DesktopOutput[];
}

export interface ClientOptions {
  translate?: (text: string) => string;
  onControls?: (state: ControlState) => void;
  onToolsOpen?: (open: boolean, focus: boolean) => void;
  isToolsOpen?: () => boolean;
}

export interface Elements {
  desktop: HTMLCanvasElement;
  "mobile-input": HTMLTextAreaElement;
  "mobile-shortcuts": HTMLElement;
  "mobile-keyboard": HTMLButtonElement;
  viewport: HTMLElement;
  text: HTMLTextAreaElement;
  width: HTMLInputElement;
  height: HTMLInputElement;
  "auto-resolution": HTMLInputElement;
  "video-codec": HTMLSelectElement;
  "stream-strategy": HTMLSelectElement;
  "shortcut-ctrl": HTMLInputElement;
  "shortcut-alt": HTMLInputElement;
  "shortcut-shift": HTMLInputElement;
  "shortcut-super": HTMLInputElement;
  "shortcut-key": HTMLSelectElement;
  "desktop-source": HTMLSelectElement;
  "monitor-source": HTMLSelectElement;
  "keyboard-section": HTMLDetailsElement;
  "settings-section": HTMLDetailsElement;
  "clipboard-section": HTMLDetailsElement;
  "apply-size": HTMLButtonElement;
  "send-text": HTMLButtonElement;
  "clipboard-get": HTMLButtonElement;
  "clipboard-set": HTMLButtonElement;
  "paste-local": HTMLButtonElement;
  "keyboard-capture": HTMLButtonElement;
  "send-shortcut": HTMLButtonElement;
  "refresh-desktops": HTMLButtonElement;
  audio: HTMLButtonElement;
  connection: HTMLButtonElement;
  "mode-fit": HTMLButtonElement;
  "mode-actual": HTMLButtonElement;
  fullscreen: HTMLButtonElement;
  "tools-toggle": HTMLButtonElement;
  "tools-close": HTMLButtonElement;
  "clipboard-toggle": HTMLButtonElement;
  "settings-toggle": HTMLButtonElement;
  tools: HTMLElement;
  "tool-panel": HTMLElement;
  resize: HTMLFormElement;
  "desktop-controls": HTMLElement;
  "monitor-controls": HTMLElement;
  "desktop-status": HTMLElement;
  "keyboard-state": HTMLElement;
  "video-details": HTMLElement;
  performance: HTMLElement;
  message: HTMLElement;
  state: HTMLElement;
  "software-version": HTMLElement;
  "display-mode": HTMLElement;
}

declare global {
  interface Navigator {
    keyboard?: { lock: () => Promise<void>; unlock: () => void };
  }
  interface Window {
    webkitAudioContext?: typeof AudioContext;
  }
}
