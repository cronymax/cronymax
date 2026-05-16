import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { WysiwygMarkdown } from "../../components/WysiwygMarkdown";
import { docType } from "../../shells/runtime";
import { Field } from "./App";

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
  const [loaded, setLoaded] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<DocTypeDraft | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadList = useCallback(async () => {
    try {
      const res = await docType.list();
      setDocTypes((res.doc_types ?? []) as DocTypeSummary[]);
      setLoaded(true);
    } catch (err) {
      setError(`doc_type.list: ${(err as Error).message}`);
      setLoaded(true);
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
      <aside className="flex w-[200px] flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between px-2 py-1.5">
          <span className="text-xs font-semibold">Doc Types</span>
          <Button type="button" size="icon" className="h-5 w-5 text-xs" onClick={onNew} title="New doc type">
            +
          </Button>
        </div>
        <Separator />
        <ul className="flex-1 overflow-auto py-1">
          {!loaded && (
            <>
              <Skeleton className="mx-2 my-1 h-8" />
              <Skeleton className="mx-2 my-1 h-8" />
              <Skeleton className="mx-2 my-1 h-8" />
            </>
          )}
          {loaded && docTypes.length === 0 && (
            <li className="px-2 py-1 text-xs text-muted-foreground">No doc types found.</li>
          )}
          {docTypes.map((dt) => (
            <li key={dt.name}>
              <Button
                type="button"
                variant="ghost"
                onClick={() => void onSelect(dt)}
                className={
                  "flex h-auto w-full flex-col items-start px-2 py-1 text-left text-xs font-normal " +
                  (selected === dt.name && !creating
                    ? "bg-primary/15 text-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground")
                }
              >
                <span className="font-medium">{dt.name}</span>
                <span className="text-xs opacity-70">{dt.user_defined ? "user" : "built-in"}</span>
              </Button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-muted-foreground">
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
              <p className="mb-3 rounded border border-border bg-card p-2 text-xs text-muted-foreground">
                Built-in doc types are read-only. Create a user doc type to define your own document structure.
              </p>
            )}
            <Field label="Name (file basename)">
              <Input
                className="h-7 text-xs"
                value={draft.name}
                disabled={!creating}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="my-doc-type"
              />
              {!creating && <p className="mt-1 text-xs text-muted-foreground">Rename by deleting and recreating.</p>}
            </Field>
            <Field label="Display name">
              <Input
                className="h-7 text-xs"
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
                <Button type="button" size="sm" onClick={() => void onSave()} disabled={busy}>
                  {creating ? "Create" : "Save"}
                </Button>
                {!creating && selectedIsUserDefined && (
                  <Button type="button" size="sm" variant="destructive" onClick={() => void onDelete()} disabled={busy}>
                    Delete
                  </Button>
                )}
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setDraft(null);
                    setCreating(false);
                    setSelected(null);
                    setError(null);
                  }}
                >
                  Cancel
                </Button>
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
