import type { ReactNode } from "react";

const MARKS: [RegExp, (text: ReactNode, key: number) => ReactNode][] = [
  [/```([\s\S]+?)```/, (text, key) => <pre key={key}>{text}</pre>],
  [/`([^`\n]+)`/, (text, key) => <code key={key}>{text}</code>],
  [/(?<![\w*])\*([^*\n]+)\*(?![\w*])/, (text, key) => <strong key={key}>{text}</strong>],
  [/(?<![\w_])_([^_\n]+)_(?![\w_])/, (text, key) => <em key={key}>{text}</em>],
  [/(?<![\w~])~([^~\n]+)~(?![\w~])/, (text, key) => <s key={key}>{text}</s>],
];

/** Draws WhatsApp's own markup the way WhatsApp does. */
export function formatted(text: string): ReactNode[] {
  const first = MARKS.map(([pattern, wrap]) => ({ match: pattern.exec(text), wrap }))
    .filter(({ match }) => match)
    .sort((a, b) => a.match!.index - b.match!.index)[0];
  if (!first) return text ? [text] : [];
  const match = first.match!;
  const inner = match[0].startsWith("`") ? [match[1]] : formatted(match[1]);
  return [
    text.slice(0, match.index),
    first.wrap(inner, match.index),
    ...formatted(text.slice(match.index + match[0].length)),
  ];
}
