import { ArrowRight } from "@carbon/icons-react";
import { useEffect, useState } from "react";
import { Navigate, useLocation, useNavigate } from "react-router";
import { LiminaMark } from "../components/ui/Logo";
import { ApiError, api, session } from "../lib/api";
import { keys, queryClient, useAuthConfig } from "../lib/queries";
import type { AuthMode } from "../lib/types";

const PROVIDER_ERRORS: Record<string, { title: string; body: string }> = {
  permission: {
    title: "You don’t have access yet",
    body: "Your account signed in, but it hasn’t been granted access to this Limina runtime. Ask an administrator to add the permission, then sign in again.",
  },
  organization: {
    title: "Wrong organization",
    body: "This Limina runtime serves a different organization. Sign in with your work account for that organization.",
  },
  cancelled: { title: "Sign-in was cancelled", body: "You can try again whenever you’re ready." },
  state: { title: "That sign-in expired", body: "Sign-in links are valid for a few minutes. Please try again." },
  token: {
    title: "Your session couldn’t be verified",
    body: "WorkOS signed you in, but this Limina runtime couldn’t verify the session. An administrator should check its WorkOS configuration.",
  },
};

function Card({ children }: { children: React.ReactNode }) {
  return (
    <main className="signin">
      <div className="signin-card">
        <LiminaMark size={40} />
        {children}
      </div>
      <a className="signature" href="https://theagilemonkeys.com" target="_blank" rel="noreferrer">
        An initiative by The Agile Monkeys
      </a>
    </main>
  );
}

/** Hosted sign-in (WorkOS AuthKit): the runtime owns the session; the console never sees a token. */
function ProviderSignIn({ from }: { from: string }) {
  const { search } = useLocation();
  const params = new URLSearchParams(search);
  const error = params.get("error");
  const signedOut = params.has("signed_out");
  const loginUrl = `/auth/login?return_to=${encodeURIComponent(from)}`;
  const known = error ? PROVIDER_ERRORS[error] : undefined;

  useEffect(() => {
    // Like any SSO app: go straight to the organization's sign-in unless there is news to show.
    if (!error && !signedOut) window.location.replace(loginUrl);
  }, [error, signedOut, loginUrl]);

  const title = known?.title ?? (error ? "Sign-in didn’t complete" : signedOut ? "You’re signed out" : "Signing you in…");
  const body =
    known?.body ??
    (error
      ? "We couldn’t complete your sign-in. Please try again."
      : signedOut
        ? "Your Limina session and your organization session have ended."
        : "Taking you to your organization’s sign-in.");

  return (
    <Card>
      <div>
        <h1 className="signin-title">{title}</h1>
        <p className="muted" style={{ marginTop: 6 }} role={error ? "alert" : undefined}>
          {body}
        </p>
      </div>
      {!error && !signedOut ? (
        <span className="spinner" aria-label="Redirecting" />
      ) : (
        <a className="btn btn-primary btn-lg" href={loginUrl} style={{ width: "100%" }}>
          {signedOut ? "Sign in" : "Try again"} <ArrowRight size={16} />
        </a>
      )}
      <p className="field-hint">Sign-in is handled by WorkOS. Limina never sees your password.</p>
    </Card>
  );
}

/** Token sign-in for local runtimes and generic OIDC deployments. */
function TokenSignIn({ from, mode }: { from: string; mode: AuthMode }) {
  const navigate = useNavigate();
  const [token, setToken] = useState("");
  const [name, setName] = useState(session.actor ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    const previous = session.token;
    session.set({ token: token.trim() || null, actor: name });
    try {
      const me = await api.me();
      queryClient.clear();
      queryClient.setQueryData(keys.me, me);
      navigate(from, { replace: true });
    } catch (caught) {
      session.set({ token: previous });
      setError(
        caught instanceof ApiError && caught.status === 401
          ? "That token wasn’t accepted by this runtime."
          : (caught as Error).message,
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="signin">
      <form className="signin-card" onSubmit={submit}>
        <LiminaMark size={40} />
        <div>
          <h1 className="signin-title">Sign in to Limina</h1>
          <p className="muted" style={{ marginTop: 6 }}>
            {mode === "oidc"
              ? "Paste an access token issued by your identity provider."
              : "Use the access token configured for this runtime."}
          </p>
        </div>
        {mode === "token" && (
          <div className="notice" style={{ width: "100%" }}>
            <div className="notice-body" style={{ margin: 0 }}>
              On a local deployment it’s <code className="mono">LIMINA_ADMIN_API_TOKEN</code> in the{" "}
              <code className="mono">.env</code> file next to <code className="mono">compose.yaml</code>. Or run{" "}
              <code className="mono">make ui-open</code> there to open Limina already signed in.
            </div>
          </div>
        )}
        <div className="form-grid" style={{ gap: "var(--space-4)", width: "100%" }}>
          <label className="field">
            <span className="field-label">Access token</span>
            <input
              className="input"
              type="password"
              autoComplete="current-password"
              autoFocus
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder="Paste your access token"
            />
          </label>
          {mode === "token" && (
            <label className="field">
              <span className="field-label">Your name</span>
              <input
                className="input"
                autoComplete="name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Shown on the messages you send"
              />
              <span className="field-hint">Used for attribution on shared-token runtimes.</span>
            </label>
          )}
          {error && (
            <div className="notice" data-tone="critical" role="alert">
              {error}
            </div>
          )}
          <button className="btn btn-primary btn-lg" type="submit" disabled={busy}>
            {busy ? <span className="spinner" /> : null}
            Continue <ArrowRight size={16} />
          </button>
        </div>
      </form>
      <a className="signature" href="https://theagilemonkeys.com" target="_blank" rel="noreferrer">
        An initiative by The Agile Monkeys
      </a>
    </main>
  );
}

export function SignIn() {
  const location = useLocation();
  const config = useAuthConfig();
  const requested = (location.state as { from?: string } | null)?.from ?? "/";
  const from = requested.startsWith("/signin") ? "/" : requested;

  useEffect(() => {
    document.title = "Sign in · Limina";
  }, []);

  if (config.isPending) {
    return (
      <Card>
        <span className="spinner" aria-label="Loading" />
      </Card>
    );
  }
  const mode = config.data?.mode ?? "token";
  if (mode === "open") return <Navigate to={from} replace />;
  if (mode === "workos") return <ProviderSignIn from={from} />;
  return <TokenSignIn from={from} mode={mode} />;
}
