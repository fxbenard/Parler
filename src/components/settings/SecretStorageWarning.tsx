import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";

/**
 * Renders a warning when LLM API keys are not stored in the OS keychain
 * (plaintext fallback, e.g. Linux without Secret Service). Renders nothing
 * when the keychain is available.
 */
export const SecretStorageWarning: React.FC = () => {
  const { t } = useTranslation();
  const [keychainAvailable, setKeychainAvailable] = useState<boolean | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    commands
      .getSecretsBackendStatus()
      .then((available) => {
        if (!cancelled) setKeychainAvailable(available);
      })
      .catch(() => {
        if (!cancelled) setKeychainAvailable(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (keychainAvailable !== false) {
    return null;
  }

  return (
    <div
      role="alert"
      className="rounded-md border border-amber-400 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-500/40 dark:bg-amber-950/40 dark:text-amber-200"
    >
      <p className="font-medium">
        {t("settings.advanced.secretStorageWarning.title")}
      </p>
      <p className="mt-1 opacity-90">
        {t("settings.advanced.secretStorageWarning.description")}
      </p>
    </div>
  );
};
