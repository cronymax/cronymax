import { Check, Lock, Plus, Save, Trash2, TriangleAlert, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Caption, ErrorText } from "@/components/ui/typography";
import { shells } from "../../shells/bridge";
import { Field } from "./App";

// ── Profiles tab ─────────────────────────────────────────────────────────
interface ProfileRecord {
  id: string;
  name: string;
  memory_id: string;
  allow_network: boolean;
  extra_read_paths: string[];
  extra_write_paths: string[];
  extra_deny_paths: string[];
}
/** Inline edit form for a single profile. */
function ProfileForm({
  initial,
  onSave,
  onDelete,
  onCancel,
  isDefault,
}: {
  initial: ProfileRecord;
  onSave: (r: ProfileRecord) => Promise<void>;
  onDelete?: () => Promise<void>;
  onCancel: () => void;
  isDefault: boolean;
}) {
  const [name, setName] = useState(initial.name);
  const [memoryId, setMemoryId] = useState(initial.memory_id || initial.id || "default");
  const [allowNet, setAllowNet] = useState(initial.allow_network);
  const [reads, setReads] = useState(initial.extra_read_paths.join("\n"));
  const [writes, setWrites] = useState(initial.extra_write_paths.join("\n"));
  const [denies, setDenies] = useState(initial.extra_deny_paths.join("\n"));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [missingPaths, setMissingPaths] = useState<string[]>([]);

  const splitPaths = (s: string) =>
    s
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);

  // Validate paths whenever the textarea values change.
  useEffect(() => {
    const allPaths = [...splitPaths(reads), ...splitPaths(writes), ...splitPaths(denies)];
    if (allPaths.length === 0) {
      setMissingPaths([]);
      return;
    }
    void shells.browser.profiles
      .check_paths({ paths: allPaths })
      .then(({ missing }) => setMissingPaths(missing))
      .catch(() => setMissingPaths([]));
  }, [reads, writes, denies]);

  const handleSave = async () => {
    if (!name.trim()) {
      setErr("Name is required.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await onSave({
        ...initial,
        name: name.trim(),
        memory_id: (memoryId || initial.id || "default").trim(),
        allow_network: allowNet,
        extra_read_paths: splitPaths(reads),
        extra_write_paths: splitPaths(writes),
        extra_deny_paths: splitPaths(denies),
      });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!onDelete) return;
    setBusy(true);
    setErr(null);
    try {
      await onDelete();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const taCls = "min-h-[80px] resize-y font-mono text-xs";

  return (
    <div className="mt-2 rounded-md border border-border bg-card p-3 text-xs">
      <Field label="Profile name">
        <Input
          className="h-7 text-xs"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Restricted"
          disabled={isDefault}
        />
        {isDefault && <Caption className="mt-1">The default profile name cannot be changed.</Caption>}
      </Field>
      <Field label="Memory ID">
        <Input
          className="h-7 text-xs"
          value={memoryId}
          onChange={(e) => setMemoryId(e.target.value)}
          placeholder={initial.id || "default"}
          spellCheck={false}
        />
        <Caption className="mt-1">Runtime memory path uses this ID: cronymax/Memories/&lt;memory_id&gt;</Caption>
      </Field>
      <Field label="Network">
        <div className="flex items-center gap-2">
          <Checkbox
            id="profile-allow-net"
            checked={allowNet}
            onCheckedChange={(checked) => setAllowNet(checked === true)}
          />
          <label htmlFor="profile-allow-net" className="cursor-pointer text-xs">
            Allow outbound network access
          </label>
        </div>
      </Field>
      <Field label="Extra readable paths (one per line)">
        <Textarea
          className={taCls}
          value={reads}
          onChange={(e) => setReads(e.target.value)}
          placeholder="/Users/me/datasets"
          spellCheck={false}
        />
      </Field>
      <Field label="Extra writable paths (one per line)">
        <Textarea
          className={taCls}
          value={writes}
          onChange={(e) => setWrites(e.target.value)}
          placeholder="/Users/me/scratch"
          spellCheck={false}
        />
      </Field>
      <Field label="Extra denied paths (one per line)">
        <Textarea
          className={taCls}
          value={denies}
          onChange={(e) => setDenies(e.target.value)}
          placeholder="/Users/me/secrets"
          spellCheck={false}
        />
      </Field>
      {missingPaths.length > 0 && (
        <Alert variant="destructive" className="mb-3">
          <TriangleAlert />
          <AlertTitle>Paths not found on disk</AlertTitle>
          <AlertDescription>
            <ul className="list-inside list-disc font-mono">
              {missingPaths.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}
      {err && <ErrorText className="mb-2">{err}</ErrorText>}
      <div className="flex items-center gap-2">
        <Button type="button" onClick={() => void handleSave()} disabled={busy}>
          <Save />
          Save
        </Button>
        <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
          <X />
          Cancel
        </Button>
        {onDelete && !isDefault && (
          <Button
            type="button"
            variant="destructive"
            className="ml-auto"
            onClick={() => void handleDelete()}
            disabled={busy}
          >
            <Trash2 />
            Delete
          </Button>
        )}
        {isDefault && (
          <span
            className="ml-auto inline-flex items-center gap-1 text-xs text-muted-foreground"
            title="The default profile cannot be deleted"
          >
            <Lock className="size-3" /> Cannot delete default
          </span>
        )}
      </div>
    </div>
  );
}
/** New-profile creation form. */
function NewProfileForm({ onCreated }: { onCreated: () => void }) {
  const [open, setOpen] = useState(false);

  if (!open) {
    return (
      <Button type="button" variant="outline" className="mt-3 border-dashed" onClick={() => setOpen(true)}>
        <Plus />
        New profile
      </Button>
    );
  }

  const blank: ProfileRecord = {
    id: "",
    name: "",
    memory_id: "default",
    allow_network: true,
    extra_read_paths: [],
    extra_write_paths: [],
    extra_deny_paths: [],
  };

  return (
    <ProfileForm
      initial={blank}
      isDefault={false}
      onSave={async (r) => {
        await shells.browser.profiles.create({
          name: r.name,
          memory_id: r.memory_id,
          allow_network: r.allow_network,
          extra_read_paths: r.extra_read_paths,
          extra_write_paths: r.extra_write_paths,
          extra_deny_paths: r.extra_deny_paths,
        });
        setOpen(false);
        onCreated();
      }}
      onCancel={() => setOpen(false)}
    />
  );
}
export function ProfilesTab() {
  const [profiles, setProfiles] = useState<ProfileRecord[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setMsg(null);
    try {
      const res = (await shells.browser.profiles.list()) as ProfileRecord[];
      setProfiles(res);
    } catch (e) {
      setMsg(`load failed: ${(e as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleSave = async (r: ProfileRecord) => {
    setBusy(true);
    try {
      await shells.browser.profiles.update({
        id: r.id,
        name: r.name,
        memory_id: r.memory_id,
        allow_network: r.allow_network,
        extra_read_paths: r.extra_read_paths,
        extra_write_paths: r.extra_write_paths,
        extra_deny_paths: r.extra_deny_paths,
      });
      setExpandedId(null);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (id: string) => {
    setBusy(true);
    try {
      await shells.browser.profiles.delete({ id });
      setExpandedId(null);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full overflow-auto p-4">
      <Caption className="mb-3">
        Named sandbox profiles are stored in <code>~/.cronymax/profiles/</code>. Assign a profile to a workspace when
        opening a folder.
      </Caption>
      {msg && <ErrorText className="mb-3">{msg}</ErrorText>}
      <div className="flex max-w-[600px] flex-col gap-2">
        {profiles.map((p) => (
          <div key={p.id}>
            <Button
              type="button"
              variant="outline"
              onClick={() => setExpandedId((prev) => (prev === p.id ? null : p.id))}
              className="w-full justify-between"
            >
              <span>{p.name}</span>
              <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                <span className="inline-flex items-center gap-1">
                  {p.allow_network ? <Check className="size-3" /> : <X className="size-3" />}
                  network
                </span>
                <span aria-hidden="true">·</span>
                {p.id === "default" ? (
                  <span className="inline-flex items-center gap-1">
                    <Lock className="size-3" />
                    default
                  </span>
                ) : (
                  <span>{p.id}</span>
                )}
              </span>
            </Button>
            {expandedId === p.id && (
              <ProfileForm
                initial={p}
                isDefault={p.id === "default"}
                onSave={handleSave}
                onDelete={() => handleDelete(p.id)}
                onCancel={() => setExpandedId(null)}
              />
            )}
          </div>
        ))}
      </div>
      <NewProfileForm onCreated={() => void reload()} />
      {busy && <Caption className="mt-3">Saving…</Caption>}
    </div>
  );
}
