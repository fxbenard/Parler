import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface ConvertNumbersProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ConvertNumbers: React.FC<ConvertNumbersProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const convertNumbers = getSetting("convert_numbers_enabled") || false;

    return (
      <ToggleSwitch
        checked={convertNumbers}
        onChange={(enabled) => updateSetting("convert_numbers_enabled", enabled)}
        isUpdating={isUpdating("convert_numbers_enabled")}
        label={t("settings.advanced.convertNumbers.label")}
        description={t("settings.advanced.convertNumbers.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
