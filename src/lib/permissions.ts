import { commands } from "@/bindings";
import { checkAccessibilityPermission } from "tauri-plugin-macos-permissions-api";

/**
 * Returns true when macOS considers this app trusted for accessibility input.
 *
 * AXIsProcessTrusted (behind `checkAccessibilityPermission`) can report a
 * stale `false` after an update or a TCC reset even though the toggle is on.
 * Falling back to actually initializing Enigo distinguishes "still denied"
 * from "reported false but working".
 */
export const checkMacOSAccessibilityReady = async (): Promise<boolean> => {
  try {
    if (await checkAccessibilityPermission()) {
      return true;
    }
  } catch (error) {
    console.warn("Failed to query macOS accessibility permission:", error);
  }

  const enigoResult = await commands.initializeEnigo();
  return enigoResult.status === "ok";
};

/** Initializes Enigo then global shortcuts, surfacing the first failure. */
export const initializeMacOSAccessibilitySystems = async (): Promise<void> => {
  const enigoResult = await commands.initializeEnigo();
  if (enigoResult.status === "error") {
    throw new Error(enigoResult.error);
  }

  const shortcutsResult = await commands.initializeShortcuts();
  if (shortcutsResult.status === "error") {
    throw new Error(shortcutsResult.error);
  }
};
