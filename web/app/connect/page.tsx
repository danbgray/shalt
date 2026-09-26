"use client";

import { useEffect, useState } from "react";

type Repo = { name: string; url: string; clone: string; private: boolean; description: string };

export default function Connect() {
  const [login, setLogin] = useState<string | null>(null);
  const [repos, setRepos] = useState<Repo[]>([]);
  const [err, setErr] = useState("");

  useEffect(() => {
    fetch("/api/me")
      .then((r) => r.json())
      .then((j) => {
        setLogin(j.login || null);
        if (j.login) return fetch("/api/github/repos");
        return null;
      })
      .then((r) => r && r.json())
      .then((j) => {
        if (j && j.repos) setRepos(j.repos);
        if (j && j.error) setErr(j.error);
      })
      .catch((e) => setErr(String(e)));
  }, []);

  return (
    <main>
      <p className="mark">Shalt</p>
      <h1>Connect GitHub</h1>
      {!login ? (
        <>
          <p className="lede">Sign in with GitHub, then wrap a repo with the CLI.</p>
          <div className="row">
            <a className="btn" href="/api/github/start?next=/connect">
              Continue with GitHub
            </a>
          </div>
        </>
      ) : (
        <>
          <p className="lede">
            Signed in as <code>{login}</code>. Onboard from a machine that has <code>shalt</code>:
          </p>
          {err ? <p>{err}</p> : null}
          <ul className="repos">
            {repos.map((r) => (
              <li key={r.name}>
                <strong>{r.name}</strong>
                {r.private ? " · private" : ""}
                <code>shalt onboard {r.url.replace("https://", "")}</code>
              </li>
            ))}
          </ul>
        </>
      )}
      <p className="note">
        Agents keep using the CLI. The Space does not replace <code>shalt</code>; it is where
        humans meet the work.
      </p>
    </main>
  );
}
