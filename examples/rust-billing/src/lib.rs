//! Billing rules.

/// Which reminder is due for an invoice this many days overdue.
pub fn reminder_stage(days_overdue: u32) -> &'static str {
    if days_overdue >= 60 {
        "final"
    } else if days_overdue >= 30 {
        "firm"
    } else if days_overdue >= 1 {
        "gentle"
    } else {
        "none"
    }
}

/// Format an amount held in minor units for the customer's currency.
pub fn format_amount(cents: i64, code: &str) -> String {
    let (symbol, minor) = match code {
        "USD" => ("$", 2),
        "EUR" => ("\u{20ac}", 2),
        "JPY" => ("\u{a5}", 0),
        _ => ("", 2),
    };
    if minor == 0 {
        // whole units, rounded half away from zero rather than truncated
        let whole = (cents + 50) / 100;
        format!("{}{}", symbol, group(whole))
    } else {
        format!("{}{}.{:02}", symbol, group(cents / 100), cents % 100)
    }
}

fn group(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{}", out)
    } else {
        out
    }
}
