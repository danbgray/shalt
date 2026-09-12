"""Escalation schedule for overdue invoices."""

THRESHOLDS = ((60, "final"), (30, "firm"), (1, "gentle"))


def reminder_stage(days_overdue):
    for floor, stage in THRESHOLDS:
        if days_overdue >= floor:
            return stage
    return "none"
