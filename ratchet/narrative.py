"""User stories, and the graph hiding inside them.

A Gherkin feature's description block conventionally holds a user story:

    Feature: Invoice totals

      As a billing clerk
      I want invoice totals computed exactly
      So that customers are never billed the wrong amount

That sentence already contains a use case diagram. "As a <actor>" is the actor, "I want
<capability>" is the use case, and the fact that they appear in the same story is the
association between them. Nothing else needs authoring, and because the diagram is derived
rather than drawn, it cannot drift away from the spec.

Clauses are parsed independently so the usual orderings all work:

    As a X / I want Y / So that Z
    As a X / In order to Z / I want Y
    In order to Z / As a X / I want Y
"""
from __future__ import annotations

import re
from dataclasses import dataclass

ACTOR_RE = re.compile(r"^\s*As\s+(?:an?|the)\s+(?P<v>.+?)\s*[,.]?\s*$", re.I | re.M)
WANT_RE = re.compile(
    r"^\s*I\s+(?:want|need|would\s+like|can|do)\s+(?:to\s+)?(?P<v>.+?)\s*[,.]?\s*$", re.I | re.M)
BENEFIT_RE = re.compile(
    r"^\s*(?:So\s+that|In\s+order\s+to)\s+(?P<v>.+?)\s*[,.]?\s*$", re.I | re.M)


@dataclass
class Story:
    actor: str = ""
    capability: str = ""
    benefit: str = ""
    raw: str = ""

    @property
    def complete(self) -> bool:
        return bool(self.actor and self.capability)

    @property
    def missing(self) -> list[str]:
        out = []
        if not self.actor:
            out.append("actor (\"As a ...\")")
        if not self.capability:
            out.append("capability (\"I want ...\")")
        if not self.benefit:
            out.append("benefit (\"So that ...\")")
        return out

    def one_line(self) -> str:
        if not self.complete:
            return self.raw.strip().splitlines()[0] if self.raw.strip() else ""
        s = f"As a {self.actor}, I want {self.capability}"
        return s + (f", so that {self.benefit}" if self.benefit else "")


def parse_story(description: str) -> Story:
    """Pull the user story out of a feature description block. Tolerant: a description that
    is not a user story yields an empty Story rather than an error, and `missing` says what a
    partial one lacks so `ratchet verify` can nag about it."""
    text = description or ""
    def grab(rx):
        m = rx.search(text)
        return m.group("v").strip() if m else ""
    return Story(actor=grab(ACTOR_RE), capability=grab(WANT_RE),
                 benefit=grab(BENEFIT_RE), raw=text)


def slug(value: str, fallback: str = "unassigned") -> str:
    s = re.sub(r"[^a-z0-9]+", "-", (value or "").lower()).strip("-")
    return s or fallback
