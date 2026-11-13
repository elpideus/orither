const HINT = /\b(code|codes|codice|otp|pin|passcode|password|token|verification|verifica|verify|c[oó]digo|autenticazione|login)\b/i;
const SPACED = /(?<![\d+])\d{3}[ -]\d{3}(?![\d])/;
const DIGITS = /(?<![\d+/.,:-])\d{4,8}(?![\d]|[/.,:-]\d)/;
const MIXED = /\b(?=[A-Z0-9-]*\d)(?=[A-Z0-9-]*[A-Z])[A-Z0-9]{1,3}-?[A-Z0-9]{3,6}\b/;

/** Finds the one-time code in a message, if it reads like it carries one. */
export function findCode(text: string): string | null {
  const hint = HINT.exec(text);
  if (!hint) return null;
  const after = text.slice(hint.index);
  for (const pattern of [DIGITS, SPACED, MIXED]) {
    const match = pattern.exec(after) ?? pattern.exec(text);
    if (match) return match[0].replace(" ", "");
  }
  return null;
}
