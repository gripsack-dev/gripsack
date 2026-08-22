/** Activation intents — declared, translated by adapters (0001 §3.8). */

export type Trigger = "post_link" | "post_activate" | "on_remove";

export type Intent =
  | { kind: "service"; trigger: Trigger; name: string; user: boolean }
  | { kind: "fonts"; trigger: Trigger }
  | { kind: "desktop_entry"; trigger: Trigger }
  | { kind: "custom_shell"; trigger: Trigger; script: string };

export const service = (
  name: string,
  user = true,
  trigger: Trigger = "post_activate",
): Intent => ({ kind: "service", trigger, name, user });

export const fonts = (trigger: Trigger = "post_link"): Intent => ({
  kind: "fonts",
  trigger,
});

export const desktopEntry = (trigger: Trigger = "post_link"): Intent => ({
  kind: "desktop_entry",
  trigger,
});

/** Escape hatch — flagged, shown by `plan`. */
export const customHook = (script: string, trigger: Trigger = "post_activate"): Intent => ({
  kind: "custom_shell",
  trigger,
  script,
});
