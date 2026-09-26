export default function Home() {
  return (
    <main>
      <p className="mark">Shalt</p>
      <h1>Bring the repo. Keep the CLI.</h1>
      <p className="lede">
        Connect GitHub. Agents keep talking to <code>shalt</code>. The spec still wins.
      </p>
      <p>
        Two doors, on purpose. <strong>GitHub</strong> is how a Space gets a real tree.
        The <strong>CLI</strong> is how Hermes, Claude, Grok, and you drive the loop —
        they shell out to <code>shalt</code>, they do not replace it.
      </p>
      <div className="row">
        <a className="btn" href="/connect">
          Continue with GitHub
        </a>
        <a className="ghost" href="/desk">
          Open the desk
        </a>
      </div>
      <p className="note">
        <code>shalt login</code> signs this machine into shalt.dev via GitHub.
        <code>shalt onboard github.com/org/repo</code> clones and wraps it, with or without
        a browser. Apex <code>shalt.dev</code>.
      </p>
    </main>
  );
}
