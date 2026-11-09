import type { ReactNode } from "react";

const BULLET = /^\s*[-*]\s+(.*)$/;

/** Shows a short model answer: paragraphs, and "- " lines as a list. */
export function AnswerText({ text, className }: { text: string; className?: string }) {
  const blocks: ReactNode[] = [];
  let items: string[] = [];
  const flush = () => {
    if (items.length) blocks.push(<ul key={`ul-${blocks.length}`}>{items.map((t, i) => <li key={i}>{t}</li>)}</ul>);
    items = [];
  };
  for (const line of text.split("\n")) {
    const bullet = line.match(BULLET);
    if (bullet) {
      items.push(bullet[1]);
    } else {
      flush();
      if (line.trim()) blocks.push(<p key={`p-${blocks.length}`}>{line.trim()}</p>);
    }
  }
  flush();
  return <div className={className}>{blocks}</div>;
}
