import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";

type Props = { initial: string; onChange: (text: string) => void };

function toHtml(text: string): string {
  const escape = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return text.split("\n").map((line) => `<p>${escape(line)}</p>`).join("");
}

/** The rough notes editor. It reports plain text with one line for each block. */
export function Notepad({ initial, onChange }: Props) {
  const editor = useEditor({
    extensions: [StarterKit],
    content: initial ? toHtml(initial) : "",
    editorProps: { attributes: { "aria-label": "Rough notes", class: "notepad" } },
    onUpdate: ({ editor: e }) => onChange(e.getText({ blockSeparator: "\n" })),
  });
  return <EditorContent editor={editor} />;
}
