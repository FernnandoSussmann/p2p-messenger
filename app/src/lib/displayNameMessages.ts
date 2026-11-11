import type { DisplayNameKind } from "../identity";

export const displayNameMessages: Record<DisplayNameKind, string> = {
  Empty: "Display name cannot be empty.",
  TooLong: "Display name is too long.",
  ControlCharacter: "Display name cannot contain control characters or line breaks.",
  BidiOverride: "Display name cannot contain text-direction override characters.",
  InvisibleCharacter:
    "Display name cannot contain invisible characters (such as zero-width spaces or joiners).",
};
