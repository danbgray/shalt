"""Workspace configuration -- what makes shalt language-agnostic.

The zone model, the scenario ledger, identity and the guards are all language-neutral already.
The only Python-specific part was the runner. This file moves that out into configuration: a
runner is a command line plus the report format it produces.

Scenario binding is by `@rid:` tag, and a tag survives into every Cucumber-family report, so the
same ledger works for pytest-bdd, cucumber-js, cucumber-jvm, godog, Reqnroll or anything else
that can emit Cucumber JSON or Cucumber Messages.
"""
from __future__ import annotations

import shlex
from dataclasses import dataclass, field
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None

CONFIG_NAME = "shalt.toml"

# Formats shalt can read back from a test run.
FORMATS = ("shalt", "cucumber-json", "cucumber-messages")


@dataclass
class Preset:
    label: str
    command: str
    format: str
    report: str
    src: str = "src"
    steps: str = "steps"
    note: str = ""


# Starting points, not guarantees -- each needs its own toolchain present. `shalt init --stack`
# writes one of these into shalt.toml for editing.
PRESETS: dict[str, Preset] = {
    "python": Preset(
        "Python / pytest-bdd",
        "python -m pytest -q --no-header -p shalt.pytest_plugin "
        "--shalt-spec={spec} --shalt-report={report} {steps}",
        "shalt", ".shalt/last_run.json",
        note="Native: binds by rid through shalt's own pytest plugin."),
    "javascript": Preset(
        "JavaScript / cucumber-js",
        "npx cucumber-js {spec} --require {steps} --format message:{report}",
        "cucumber-messages", ".shalt/messages.ndjson",
        src="src", steps="steps",
        note="Needs @cucumber/cucumber installed in the workspace."),
    "go": Preset(
        "Go / godog",
        "godog run --format=cucumber --paths={spec} > {report}",
        "cucumber-json", ".shalt/cucumber.json",
        src="internal", steps="features",
        note="godog writes the report to stdout, hence the redirect."),
    "java": Preset(
        "Java / cucumber-jvm",
        "mvn -q test -Dcucumber.features={spec} "
        "-Dcucumber.plugin=json:{report}",
        "cucumber-json", "target/cucumber.json",
        src="src/main/java", steps="src/test/java",
        note="Assumes a Maven project with cucumber-junit wired up."),
    "ruby": Preset(
        "Ruby / cucumber",
        "bundle exec cucumber {spec} -r {steps} --format json --out {report}",
        "cucumber-json", ".shalt/cucumber.json",
        src="lib", steps="features/step_definitions"),
    "dotnet": Preset(
        ".NET / Reqnroll",
        "dotnet test -- Reqnroll.Output.Cucumber={report}",
        "cucumber-json", ".shalt/cucumber.json",
        src="src", steps="Tests"),
}


@dataclass
class Config:
    command: str = PRESETS["python"].command
    format: str = "shalt"
    report: str = ".shalt/last_run.json"
    src: str = "src"
    steps: str = "steps"
    stack: str = "python"
    name: str = ""
    timeout: int = 900
    env: dict[str, str] = field(default_factory=dict)

    @classmethod
    def load(cls, root: Path) -> "Config":
        path = Path(root) / CONFIG_NAME
        if not path.exists():
            return cls()
        if tomllib is None:  # pragma: no cover
            raise RuntimeError("reading shalt.toml needs Python 3.11+ (tomllib)")
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
        proj, runner, zones = (raw.get("project", {}), raw.get("runner", {}),
                              raw.get("zones", {}))
        cfg = cls(
            command=runner.get("command", cls.command),
            format=runner.get("format", "shalt"),
            report=runner.get("report", cls.report),
            src=zones.get("src", "src"),
            steps=zones.get("steps", "steps"),
            stack=proj.get("stack", "python"),
            name=proj.get("name", ""),
            timeout=int(runner.get("timeout", 900)),
            env={str(k): str(v) for k, v in (runner.get("env") or {}).items()},
        )
        if cfg.format not in FORMATS:
            raise ValueError(
                f"unknown runner format {cfg.format!r} in {CONFIG_NAME}; "
                f"expected one of {', '.join(FORMATS)}")
        return cfg

    def argv(self, root: Path) -> list[str]:
        """The runner command with {spec} {steps} {src} {report} {root} substituted."""
        filled = self.command.format(
            spec=str(Path(root) / "spec"), steps=str(Path(root) / self.steps),
            src=str(Path(root) / self.src), report=str(Path(root) / self.report),
            root=str(root))
        return shlex.split(filled)

    @property
    def uses_shell(self) -> bool:
        """A preset that redirects or pipes has to go through a shell."""
        return any(ch in self.command for ch in ("|", ">", "&&"))


def write_config(root: Path, stack: str, name: str = "") -> Preset:
    if stack not in PRESETS:
        raise ValueError(f"unknown stack {stack!r}; expected one of {', '.join(PRESETS)}")
    p = PRESETS[stack]
    body = f'''# shalt workspace configuration
#
# shalt is language-agnostic: the zone model, the ledger, scenario identity and the write
# guards are all language-neutral. Only the runner is not, so it lives here.
#
# Scenario binding is by the @rid: tag stamped into each scenario at approval, and that tag
# survives into every Cucumber-family report -- so any runner that emits Cucumber JSON or
# Cucumber Messages works without further code.

[project]
name = "{name or Path(root).name}"
stack = "{stack}"                     # {p.label}

[zones]
steps = "{p.steps}"                   # step definitions; written by the stepwright only
src = "{p.src}"                       # implementation; written by the implementer only

[runner]
# {p.note or "Placeholders: {spec} {steps} {src} {report} {root}"}
command = "{p.command}"
format = "{p.format}"                 # shalt | cucumber-json | cucumber-messages
report = "{p.report}"
timeout = 900
'''
    (Path(root) / CONFIG_NAME).write_text(body, encoding="utf-8")
    return p
