/** Orither's mark: a ring with a smaller device in orbit. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" aria-hidden="true">
      <circle cx="23" cy="25" r="16.5" fill="none" stroke="var(--accent)" strokeWidth="7" />
      <circle cx="38.5" cy="9.5" r="6.5" fill="var(--accent)" stroke="var(--bg)" strokeWidth="3" />
    </svg>
  );
}
