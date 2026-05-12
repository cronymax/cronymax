import { useCallback, useEffect, useState } from "react";
import { WysiwygMarkdown } from "../../components/WysiwygMarkdown";
import { docType } from "../../shells/runtime";
import { Field, inputCls } from "./App";

// ── Doc Types tab ─────────────────────────────────────────────────────────
interface DocTypeSummary {
  name: string;
  display_name: string;
  user_defined: boolean;
}
interface DocTypeDraft {
  name: string;
  display_name: string;
  description: string;
}
const EMPTY_DOC_TYPE: DocTypeDraft = {
  name: "",
  display_name: "",
  description: "",
};
export function DocTypesTab() {
  const [docTypes, setDocTypes] = useState<DocTypeSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<DocTypeDraft | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadList = useCallback(async () => {
    try {
      const res = await docType.list();
      setDocTypes((res.doc_types ?? []) as DocTypeSummary[]);
    } catch (err) {
      setError(`doc_type.list: ${(err as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void loadList();
  }, [loadList]);

  const onSelect = useCallback(async (dt: DocTypeSummary) => {
    setSelected(dt.name);
    setCreating(false);
    setError(null);
    // Show name/display immediately while description loads
    setDraft({ name: dt.name, display_name: dt.display_name, description: "" });
    try {
      const res = (await docType.load(dt.name)) as {
        name: string;
        display_name: string;
        description: string;
      };
      setDraft({
        name: res.name,
        display_name: res.display_name,
        description: res.description,
      });
    } catch (err) {
      setError(`doc_type.load: ${(err as Error).message}`);
    }
  }, []);

  const onNew = useCallback(() => {
    setSelected(null);
    setCreating(true);
    setDraft({ ...EMPTY_DOC_TYPE });
    setError(null);
  }, []);

  const onSave = useCallback(async () => {
    if (!draft) return;
    if (!/^[A-Za-z0-9_.-]{1,64}$/.test(draft.name)) {
      setError("Name must be 1-64 chars of letters, digits, _, -, or . (no slashes).");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await docType.save(draft.name, draft.display_name || draft.name, draft.description);
      await loadList();
      setSelected(draft.name);
      setCreating(false);
    } catch (err) {
      setError(`save failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [draft, loadList]);

  const onDelete = useCallback(async () => {
    if (!selected) return;
    // eslint-disable-next-line no-alert
    if (!confirm(`Delete doc-type "${selected}" YAML file?`)) return;
    setBusy(true);
    setError(null);
    try {
      await docType.delete(selected);
      await loadList();
      setSelected(null);
      setDraft(null);
    } catch (err) {
      setError(`delete failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [selected, loadList]);

  const selectedIsUserDefined = selected != null && (docTypes.find((d) => d.name === selected)?.user_defined ?? false);

  const editable = creating || selectedIsUserDefined;

  return (
    <div className="flex h-full">
      <aside className="flex w-[200px] flex-col border-r border-cronymax-border bg-cronymax-float">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Doc Types</span>
          <button
            type="button"
            onClick={onNew}
            className="rounded bg-cronymax-primary px-1.5 py-0.5 text-xs text-white hover:opacity-90"
            title="New doc type"
          >
            +
          </button>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {docTypes.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-caption">No doc types found.</li>
          )}
          {docTypes.map((dt) => (
            <li key={dt.name}>
              <button
                type="button"
                onClick={() => void onSelect(dt)}
                className={
                  "flex w-full flex-col items-start px-2 py-1 text-left text-xs " +
                  (selected === dt.name && !creating
                    ? "bg-cronymax-primary/15 text-cronymax-title"
                    : "text-cronymax-caption hover:bg-cronymax-base hover:text-cronymax-title")
                }
              >
                <span className="font-medium">{dt.name}</span>
                <span className="text-[10px] opacity-70">{dt.user_defined ? "user" : "built-in"}</span>
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-caption">
            Select a doc type to view its Markdown description, or click <b>+</b> to create a new one. User-defined doc
            types are stored in <code>.cronymax/doc-types/&lt;name&gt;.yaml</code> and appear alongside built-ins in the
            Flow PRODUCES picker.
          </p>
        )}
        {draft && (
          <div className="max-w-[640px]">
            <h2 className="mb-3 text-sm font-semibold">
              {creating ? "New doc type" : `${editable ? "Edit" : "View"}: ${selected}`}
            </h2>
            {!creating && !selectedIsUserDefined && (
              <p className="mb-3 rounded border border-cronymax-border bg-cronymax-float p-2 text-[11px] text-cronymax-caption">
                Built-in doc types are read-only. Create a user doc type to define your own document structure.
              </p>
            )}
            <Field label="Name (file basename)">
              <input
                className={inputCls}
                value={draft.name}
                disabled={!creating}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="my-doc-type"
              />
              {!creating && (
                <p className="mt-1 text-[10px] text-cronymax-caption">Rename by deleting and recreating.</p>
              )}
            </Field>
            <Field label="Display name">
              <input
                className={inputCls}
                value={draft.display_name}
                disabled={!editable}
                onChange={(e) => setDraft({ ...draft, display_name: e.target.value })}
                placeholder="My Doc Type"
              />
            </Field>
            <Field label="Description">
              <WysiwygMarkdown
                value={draft.description}
                onChange={editable ? (v) => setDraft({ ...draft, description: v }) : undefined}
                readOnly={!editable}
                className={!editable ? "pointer-events-none opacity-60" : undefined}
              />
            </Field>
            {error && <p className="mb-3 text-xs text-red-300">{error}</p>}
            {editable && (
              <div className="mt-3 flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void onSave()}
                  disabled={busy}
                  className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
                >
                  {creating ? "Create" : "Save"}
                </button>
                {!creating && selectedIsUserDefined && (
                  <button
                    type="button"
                    onClick={() => void onDelete()}
                    disabled={busy}
                    className="rounded border border-red-500/50 bg-red-500/10 px-3 py-1 text-xs text-red-300 hover:bg-red-500/20 disabled:opacity-50"
                  >
                    Delete
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => {
                    setDraft(null);
                    setCreating(false);
                    setSelected(null);
                    setError(null);
                  }}
                  className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs hover:bg-cronymax-float"
                >
                  Cancel
                </button>
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
