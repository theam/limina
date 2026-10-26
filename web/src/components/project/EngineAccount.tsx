import { CheckmarkFilled, Copy, Launch, WarningAltFilled } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../lib/api";
import { keys, queryClient } from "../../lib/queries";
import type { CodexDeviceLogin, Engine, ProjectEngineAccount } from "../../lib/types";
import { Tag } from "../ui/Status";
import { Time } from "../ui/Time";
import { useToast } from "../ui/Toast";

/** A token from `claude setup-token` or an API key: one long word. Mirrors the server check. */
export const looksLikeClaudeToken = (value: string) => value.trim().length >= 20 && !/\s/.test(value.trim());

const METHOD: Record<string, string> = {
  "claude-login": "Claude sign-in",
  subscription: "Subscription token",
  "api-key": "API key",
  chatgpt: "ChatGPT sign-in",
};

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "the sign-in page";
  }
}

function accountsChanged(slug: string) {
  queryClient.invalidateQueries({ queryKey: keys.engineAccounts(slug) });
  queryClient.invalidateQueries({ queryKey: keys.preflight(slug) });
  queryClient.invalidateQueries({ queryKey: ["models"] });
}

export function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="command-line">
      <code>{command}</code>
      <button
        type="button"
        className="btn btn-ghost btn-icon btn-sm"
        aria-label={copied ? "Copied" : `Copy ${command}`}
        onClick={async () => {
          await navigator.clipboard?.writeText(command).catch(() => undefined);
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1500);
        }}
      >
        {copied ? <CheckmarkFilled size={16} /> : <Copy size={16} />}
      </button>
    </div>
  );
}

/** The steps to create a Claude token, plus the field to paste it into. */
export function ClaudeTokenSteps({
  token,
  onToken,
  autoFocus,
}: {
  token: string;
  onToken: (value: string) => void;
  autoFocus?: boolean;
}) {
  const invalid = token.length > 0 && !looksLikeClaudeToken(token);
  return (
    <ol className="setup-steps">
      <li>
        <span>On your computer, run</span>
        <CopyCommand command="claude setup-token" />
      </li>
      <li>
        <span>Sign in with your account in the browser it opens. The terminal then prints a long-lived token.</span>
      </li>
      <li>
        <label className="field" style={{ width: "100%" }}>
          <span>Paste the token</span>
          <input
            className="input mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus={autoFocus}
            value={token}
            aria-invalid={invalid || undefined}
            onChange={(event) => onToken(event.target.value)}
            placeholder="sk-ant-…"
          />
          <span className="field-hint" data-tone={invalid ? "critical" : undefined}>
            {invalid
              ? "That doesn’t look like a token. Copy the whole line the command prints."
              : "Stored encrypted and never shown again. An API key works too."}
          </span>
        </label>
      </li>
    </ol>
  );
}

/** Sign-in for a project's Claude account: open claude.com, sign in, paste the code it shows. */
export function useClaudeAccountLogin(slug: string, onConnected?: () => void) {
  const toast = useToast();
  const [login, setLogin] = useState<CodexDeviceLogin | null>(null);
  const connected = useRef(onConnected);
  connected.current = onConnected;

  const start = useMutation({
    mutationFn: () => api.startClaudeAccountLogin(slug),
    onSuccess: setLogin,
    onError: (error) => toast("Couldn’t start sign-in", { body: (error as Error).message, tone: "critical" }),
  });
  const submit = useMutation({
    mutationFn: (code: string) => api.submitClaudeAccountCode(slug, login!.login_id, code),
    onSuccess: (next) => {
      setLogin(next);
      if (next.status === "SUCCEEDED") {
        accountsChanged(slug);
        toast("Claude account connected", { tone: "success" });
        setLogin(null);
        connected.current?.();
      }
    },
    onError: (error) => toast("Couldn’t sign in", { body: (error as Error).message, tone: "critical" }),
  });
  const cancel = useCallback(async () => {
    if (login?.status === "PENDING") await api.cancelClaudeAccountLogin(slug, login.login_id).catch(() => undefined);
    setLogin(null);
  }, [login, slug]);

  return { login, start, submit, cancel };
}

/** The open-claude.com and paste-the-code steps of a Claude sign-in. */
function ClaudeSignIn({
  login,
  pending,
  onSubmit,
  onCancel,
}: {
  login: CodexDeviceLogin;
  pending: boolean;
  onSubmit: (code: string) => void;
  onCancel: () => void;
}) {
  const [code, setCode] = useState("");
  if (login.status === "FAILED" || login.status === "CANCELLED") {
    return (
      <div className="notice" data-tone="critical" role="alert">
        <WarningAltFilled size={16} />
        <div>{login.error ?? "Sign-in didn’t complete. Try again."}</div>
      </div>
    );
  }
  return (
    <form
      className="stack"
      style={{ gap: "var(--space-4)" }}
      onSubmit={(event) => {
        event.preventDefault();
        if (code.trim()) onSubmit(code.trim());
      }}
    >
      <ol className="setup-steps">
      <li>
        <span>Sign in with your Claude account. It opens in a new tab.</span>
        <a className="btn btn-primary btn-sm" style={{ gridColumn: 2, justifySelf: "start" }} href={login.verification_url} target="_blank" rel="noreferrer">
          Open {hostOf(login.verification_url)} <Launch size={16} />
        </a>
      </li>
      <li>
        <label className="field" style={{ width: "100%" }}>
          <span>Paste the code claude.com shows after you sign in</span>
          <input
            className="input mono"
            autoComplete="off"
            spellCheck={false}
            value={code}
            onChange={(event) => setCode(event.target.value)}
            placeholder="Authentication code"
          />
        </label>
      </li>
      </ol>
      <div className="cluster" style={{ justifyContent: "flex-end" }}>
        <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel}>
          Cancel
        </button>
        <button className="btn btn-primary btn-sm" type="submit" disabled={!code.trim() || pending}>
          {pending && <span className="spinner" />} Connect
        </button>
      </div>
    </form>
  );
}

/** Device sign-in for a project's Codex account, with polling until it settles. */
export function useCodexAccountLogin(slug: string, onConnected?: () => void) {
  const toast = useToast();
  const [login, setLogin] = useState<CodexDeviceLogin | null>(null);
  const connected = useRef(onConnected);
  connected.current = onConnected;

  const start = useMutation({
    mutationFn: () => api.startCodexAccountLogin(slug),
    onSuccess: setLogin,
    onError: (error) => toast("Couldn’t start sign-in", { body: (error as Error).message, tone: "critical" }),
  });

  useEffect(() => {
    if (!login || login.status !== "PENDING") return;
    const timer = window.setInterval(async () => {
      try {
        const next = await api.codexAccountLogin(slug, login.login_id);
        setLogin(next);
        if (next.status === "SUCCEEDED") {
          accountsChanged(slug);
          toast("Codex account connected", { tone: "success" });
          connected.current?.();
        }
      } catch {
        /* keep polling; the next tick retries */
      }
    }, 2500);
    return () => window.clearInterval(timer);
  }, [login, slug, toast]);

  const cancel = useCallback(async () => {
    if (login?.status === "PENDING") await api.cancelCodexAccountLogin(slug, login.login_id).catch(() => undefined);
    setLogin(null);
  }, [login, slug]);

  return { login, start, cancel };
}

export function DeviceCode({ login, onCancel }: { login: CodexDeviceLogin; onCancel: () => void }) {
  if (login.status === "FAILED" || login.status === "CANCELLED") {
    return (
      <div className="notice" data-tone="critical" role="alert">
        <WarningAltFilled size={16} />
        <div>{login.error ?? "Sign-in didn’t complete. Try again."}</div>
      </div>
    );
  }
  if (login.status !== "PENDING") return null;
  return (
    <div className="device-code">
      <div className="label">Enter this code</div>
      <div className="device-code-value">{login.user_code ?? "…"}</div>
      <a className="btn btn-primary" href={login.verification_url} target="_blank" rel="noreferrer">
        Open {hostOf(login.verification_url)} <Launch size={16} />
      </a>
      <div className="cluster muted" style={{ justifyContent: "center" }}>
        <span className="spinner" /> Waiting for you to confirm…
        <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function AccountState({ item }: { item: ProjectEngineAccount }) {
  if (item.account)
    return (
      <Tag tone="success">
        <CheckmarkFilled size={12} /> Connected
      </Tag>
    );
  if (item.runtime_available) return <Tag>Runtime credentials</Tag>;
  return (
    <Tag tone="warning">
      <WarningAltFilled size={12} /> Not connected
    </Tag>
  );
}

/** One engine's account for a project: who connected it, and how to connect or replace it. */
export function EngineAccountPanel({
  slug,
  item,
  owner,
  inUse,
  autoStart,
  onConnected,
}: {
  slug: string;
  item: ProjectEngineAccount;
  owner: boolean;
  inUse: boolean;
  autoStart?: boolean;
  onConnected?: () => void;
}) {
  const toast = useToast();
  const engine: Engine = item.engine;
  const account = item.account;
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [useToken, setUseToken] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const codex = useCodexAccountLogin(slug, onConnected);
  const claude = useClaudeAccountLogin(slug, () => {
    setEditing(false);
    onConnected?.();
  });
  const started = useRef(false);

  useEffect(() => {
    if (!autoStart || !owner || account || started.current) return;
    started.current = true;
    if (engine === "codex") codex.start.mutate();
    else claude.start.mutate();
  }, [autoStart, owner, engine, account, codex.start, claude.start]);

  const connect = useMutation({
    mutationFn: () => api.connectClaudeAccount(slug, token.trim()),
    onSuccess: () => {
      accountsChanged(slug);
      setToken("");
      setEditing(false);
      toast("Claude account connected", { tone: "success" });
      onConnected?.();
    },
    onError: (error) => toast("Couldn’t connect", { body: (error as Error).message, tone: "critical" }),
  });
  const disconnect = useMutation({
    mutationFn: () => api.disconnectEngineAccount(slug, engine),
    onSuccess: () => {
      accountsChanged(slug);
      setConfirming(false);
      toast(`${item.label} account disconnected`);
    },
    onError: (error) => toast("Couldn’t disconnect", { body: (error as Error).message, tone: "critical" }),
  });

  const claudeOpen = engine === "claude-code" && owner && (!account || editing);
  const showClaudeForm = claudeOpen && useToken;
  const showClaudeSignIn = claudeOpen && !useToken;
  const pendingLogin = codex.login?.status === "PENDING";
  const fallback = item.runtime_available
    ? "Without a project account, runs use the runtime’s shared credentials."
    : `The runtime has no shared ${item.label} credentials, so this project needs its own account to run on ${item.label}.`;

  return (
    <section className="panel account-panel" aria-label={`${item.label} account`}>
      <div className="panel-header">
        <span className="panel-title">
          {item.label}
          {inUse && <span className="muted" style={{ fontWeight: 400 }}> · runs this project</span>}
        </span>
        <AccountState item={item} />
      </div>
      <div className="panel-body stack" style={{ gap: "var(--space-4)" }}>
        {account && !editing && (
          <dl className="account-facts">
            <dt>Account</dt>
            <dd className="mono">{account.label ?? "Signed in"}</dd>
            <dt>Method</dt>
            <dd>{METHOD[account.method] ?? account.method}</dd>
            {account.connected_by && (
              <>
                <dt>Connected by</dt>
                <dd>
                  {account.connected_by}
                  {account.connected_at && (
                    <>
                      {" · "}
                      <Time iso={account.connected_at} className="muted" />
                    </>
                  )}
                </dd>
              </>
            )}
          </dl>
        )}
        {!account && !owner && <p className="muted">{fallback} Only project owners can connect an account.</p>}
        {showClaudeSignIn && !claude.login && (
          <p className="muted">
            Sign in with your Claude account. The sign-in belongs to this project alone and Claude keeps it fresh.{" "}
            <button type="button" className="link" onClick={() => setUseToken(true)}>
              Use a token instead
            </button>
          </p>
        )}
        {showClaudeSignIn && claude.login && (
          <ClaudeSignIn
            login={claude.login}
            pending={claude.submit.isPending}
            onSubmit={(code) => claude.submit.mutate(code)}
            onCancel={claude.cancel}
          />
        )}
        {showClaudeForm && (
          <form
            id={`connect-${engine}`}
            onSubmit={(event) => {
              event.preventDefault();
              if (looksLikeClaudeToken(token)) connect.mutate();
            }}
          >
            <ClaudeTokenSteps token={token} onToken={setToken} autoFocus={editing || autoStart} />
            <p className="field-hint" style={{ marginTop: 8 }}>
              <button type="button" className="link" onClick={() => setUseToken(false)}>
                Sign in with Claude instead
              </button>
            </p>
          </form>
        )}
        {engine === "codex" && owner && !account && !codex.login && (
          <p className="muted">
            Sign in with your ChatGPT account. The sign-in belongs to this project alone and Codex keeps it fresh.
          </p>
        )}
        {codex.login && <DeviceCode login={codex.login} onCancel={codex.cancel} />}
      </div>
      {owner && (
        <div className="panel-footer">
          <span>
            {account
              ? "Only this project’s runs use this account."
              : item.runtime_available
                ? "Optional. Until then, runs use the runtime’s shared credentials."
                : `Required to run on ${item.label}.`}
          </span>
          <span className="cluster" style={{ flexWrap: "nowrap" }}>
            {account && !editing && confirming && (
              <>
                <button type="button" className="btn btn-danger btn-sm" onClick={() => disconnect.mutate()} disabled={disconnect.isPending}>
                  {disconnect.isPending && <span className="spinner" />} Disconnect
                </button>
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirming(false)}>
                  Cancel
                </button>
              </>
            )}
            {account && !editing && !confirming && (
              <>
                {engine === "claude-code" && (
                  <button type="button" className="btn btn-sm" onClick={() => setEditing(true)}>
                    Reconnect
                  </button>
                )}
                <button type="button" className="btn btn-sm" onClick={() => setConfirming(true)}>
                  Disconnect
                </button>
              </>
            )}
            {showClaudeSignIn && !claude.login?.status?.startsWith("PENDING") && (
              <>
                {editing && (
                  <button type="button" className="btn btn-ghost btn-sm" onClick={() => setEditing(false)}>
                    Cancel
                  </button>
                )}
                <button type="button" className="btn btn-primary btn-sm" onClick={() => claude.start.mutate()} disabled={claude.start.isPending}>
                  {claude.start.isPending && <span className="spinner" />} {claude.login ? "Try again" : "Sign in with Claude"}
                </button>
              </>
            )}
            {showClaudeForm && (
              <>
                {editing && (
                  <button type="button" className="btn btn-ghost btn-sm" onClick={() => (setEditing(false), setToken(""))}>
                    Cancel
                  </button>
                )}
                <button
                  className="btn btn-primary btn-sm"
                  type="submit"
                  form={`connect-${engine}`}
                  disabled={!looksLikeClaudeToken(token) || connect.isPending}
                >
                  {connect.isPending && <span className="spinner" />} {editing ? "Save token" : "Connect"}
                </button>
              </>
            )}
            {engine === "codex" && !account && !pendingLogin && (
              <button type="button" className="btn btn-primary btn-sm" onClick={() => codex.start.mutate()} disabled={codex.start.isPending}>
                {codex.start.isPending && <span className="spinner" />} {codex.login ? "Try again" : "Sign in"}
              </button>
            )}
          </span>
        </div>
      )}
    </section>
  );
}
