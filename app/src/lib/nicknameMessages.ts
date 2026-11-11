import type { DisplayNameKind } from "../identity";

export const nicknameMessages: Record<DisplayNameKind, string> = {
  Empty: "Name cannot be empty.",
  TooLong: "Name is too long.",
  ControlCharacter: "Name cannot contain control characters or line breaks.",
  BidiOverride: "Name cannot contain text-direction override characters.",
  InvisibleCharacter:
    "Name cannot contain invisible characters (such as zero-width spaces or joiners).",
};
