import { type FormEvent, useEffect, useState } from "react";
import { ConfirmDialog } from "../../app/ConfirmDialog";
import { api, errorMessage, type Section, type Template, type TemplateInput } from "../../lib/api";
import "../library.css";

const GENERAL_ID = 1;
const EMPTY: TemplateInput = { name: "", description: "", sections: [{ name: "", hint: "" }], instructions: "" };

function usageLabel(t: Template): string {
  if (t.id === GENERAL_ID) return "Default for new notes";
  if (t.uses === 0) return "Not used yet";
  return t.uses === 1 ? "Used 1 time" : `Used ${t.uses} times`;
}

function toInput(t: Template): TemplateInput {
  return { name: t.name, description: t.description, sections: t.sections.map((s) => ({ ...s })), instructions: t.instructions };
}

const stroke = { fill: "none", stroke: "currentColor", strokeWidth: 2.2, strokeLinecap: "round" } as const;

/** 4.3 Templates: the list, and a form to edit the sections and the AI instructions. */
export function TemplatesScreen() {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [form, setForm] = useState<TemplateInput>(EMPTY);
  const [info, setInfo] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  const current = templates.find((t) => t.id === selected) ?? null;

  const load = async (select?: number | null) => {
    try {
      const list = await api.listTemplates();
      setTemplates(list);
      const pick = list.find((t) => t.id === select) ?? (select === undefined ? list[0] : undefined);
      if (pick) {
        setSelected(pick.id);
        setForm(toInput(pick));
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const choose = (t: Template) => {
    setSelected(t.id);
    setForm(toInput(t));
    setInfo(null);
    setError(null);
  };

  const startNew = () => {
    setSelected(null);
    setForm({ ...EMPTY, sections: [{ name: "", hint: "" }] });
    setInfo(null);
    setError(null);
  };

  const setSection = (i: number, patch: Partial<Section>) =>
    setForm((f) => ({ ...f, sections: f.sections.map((s, j) => (j === i ? { ...s, ...patch } : s)) }));
  const moveSection = (i: number, by: number) =>
    setForm((f) => {
      const sections = [...f.sections];
      const [item] = sections.splice(i, 1);
      sections.splice(i + by, 0, item);
      return { ...f, sections };
    });
  const removeSection = (i: number) => setForm((f) => ({ ...f, sections: f.sections.filter((_, j) => j !== i) }));
  const addSection = () => setForm((f) => ({ ...f, sections: [...f.sections, { name: "", hint: "" }] }));

  const run = async (action: () => Promise<number | void>, done: string) => {
    setError(null);
    setInfo(null);
    try {
      const id = await action();
      await load(typeof id === "number" ? id : selected);
      setInfo(done);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const save = (e: FormEvent) => {
    e.preventDefault();
    void run(() => api.saveTemplate(selected, form), "Saved.");
  };

  const remove = async () => {
    if (selected === null) return;
    setConfirming(false);
    setError(null);
    try {
      await api.deleteTemplate(selected);
      await load(GENERAL_ID);
      setInfo("Deleted. Its notes now use General.");
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="tpl-layout">
      <section aria-labelledby="tpl-title" className="tpl-list">
        <div className="tpl-list-head">
          <h1 id="tpl-title">Templates</h1>
          <button type="button" className="icon-btn-light" aria-label="New template" onClick={startNew}>
            <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" {...stroke}><path d="M12 5v14M5 12h14" /></svg>
          </button>
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 1 }}>
          {templates.map((t) => (
            <button key={t.id} type="button" className="tpl-item" aria-current={t.id === selected ? "true" : undefined} onClick={() => choose(t)}>
              <span className="name">{t.name}</span>
              <span className="muted small">{usageLabel(t)}</span>
            </button>
          ))}
        </div>
      </section>

      <form className="tpl-form" onSubmit={save} aria-label={current ? `Edit ${current.name}` : "New template"}>
        <div className="tpl-grid">
          <label htmlFor="tpl-name">Name</label>
          <input id="tpl-name" className="tpl-input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
          <label htmlFor="tpl-when">When to use</label>
          <input id="tpl-when" className="tpl-input" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
        </div>

        <fieldset className="tpl-sections">
          <legend>Sections</legend>
          {form.sections.map((s, i) => (
            <div key={i} className="section-row">
              <input aria-label={`Section ${i + 1} name`} className="tpl-input" placeholder="Section name" value={s.name} onChange={(e) => setSection(i, { name: e.target.value })} />
              <input aria-label={`Section ${i + 1} hint`} className="tpl-input" placeholder="What it holds (optional)" value={s.hint} onChange={(e) => setSection(i, { hint: e.target.value })} />
              <button type="button" className="icon-btn-light" aria-label={`Move ${s.name || `section ${i + 1}`} up`} disabled={i === 0} onClick={() => moveSection(i, -1)}>
                <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" {...stroke}><path d="M6 15l6-6 6 6" /></svg>
              </button>
              <button type="button" className="icon-btn-light" aria-label={`Move ${s.name || `section ${i + 1}`} down`} disabled={i === form.sections.length - 1} onClick={() => moveSection(i, 1)}>
                <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" {...stroke}><path d="M6 9l6 6 6-6" /></svg>
              </button>
              <button type="button" className="icon-btn-light" aria-label={`Remove ${s.name || `section ${i + 1}`}`} disabled={form.sections.length === 1} onClick={() => removeSection(i)}>
                <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" {...stroke}><path d="M6 6l12 12M18 6L6 18" /></svg>
              </button>
            </div>
          ))}
          <button type="button" className="link-btn" onClick={addSection}>
            <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" {...stroke} style={{ marginRight: 4 }}><path d="M12 5v14M5 12h14" /></svg>
            Add section
          </button>
        </fieldset>

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <label htmlFor="tpl-ai" style={{ fontWeight: 600, fontSize: 12 }}>Instructions for the AI</label>
          <textarea id="tpl-ai" rows={4} className="tpl-textarea" value={form.instructions} onChange={(e) => setForm({ ...form, instructions: e.target.value })} />
        </div>

        {error && <p role="alert" className="error-text">{error}</p>}
        {info && <p role="status" className="muted small">{info}</p>}

        <div className="row">
          <button type="submit" className="btn btn-primary">Save template</button>
          {current && <button type="button" className="btn" onClick={() => run(() => api.duplicateTemplate(current.id), "Copied.")}>Duplicate</button>}
          <span style={{ flexGrow: 1 }} />
          {current?.builtIn && <button type="button" className="btn" onClick={() => run(() => api.resetTemplate(current.id), "Reset to the built-in values.")}>Reset</button>}
          {current && !current.builtIn && <button type="button" className="btn btn-danger" onClick={() => setConfirming(true)}>Delete</button>}
        </div>
      </form>

      {confirming && current && (
        <ConfirmDialog
          title="Delete this template?"
          body={`"${current.name}" goes away. Notes that use it move to General and keep their text.`}
          confirmLabel="Delete template"
          onConfirm={remove}
          onCancel={() => setConfirming(false)}
        />
      )}
    </div>
  );
}
