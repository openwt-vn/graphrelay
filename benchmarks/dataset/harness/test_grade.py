#!/usr/bin/env python3
"""Regression tests for grade.py's verbatim-quote audit (`_quote_is_verbatim`).

No pytest dependency in this harness — plain assertions, run directly:
    python3 harness/test_grade.py

Covers a real bug found live 2026-09-28 (erpnext Q05, `graphrelay` arm): the
judge cited a quote with a "..." elision around a genuinely verbatim
surrounding passage, and the old whole-string check downgraded a correct
`covered` verdict to `missing` purely because "..." isn't literal text.
"""

from grade import _quote_is_verbatim, _strip_md

ANSWER = (
    "The function validates input and returns early on error. "
    "Later it logs the result and throws an \"Excess Material Transfer\" error "
    "if the pending quantity is exceeded."
)


def check(name: str, condition: bool) -> None:
    status = "ok" if condition else "FAIL"
    print(f"  {name} ... {status}")
    assert condition, name


def main() -> None:
    print("_quote_is_verbatim:")

    check(
        "plain real substring passes (no ellipsis, the common case)",
        _quote_is_verbatim("returns early on error", ANSWER),
    )
    check(
        "plain fabricated quote fails (no ellipsis)",
        not _quote_is_verbatim("returns early on failure", ANSWER),
    )
    check(
        "legitimate elision of a real verbatim middle passes",
        _quote_is_verbatim("validates input ... logs the result", ANSWER),
    )
    check(
        "unicode ellipsis (…) is treated the same as \"...\"",
        _quote_is_verbatim("validates input … logs the result", ANSWER),
    )
    check(
        "a fabricated middle wrapped in real edges still fails — elision "
        "must not become a license to paraphrase",
        not _quote_is_verbatim(
            "validates input ... this claim is not in the answer ... logs the result",
            ANSWER,
        ),
    )
    check(
        "out-of-order fragments fail (second fragment precedes the first "
        "in the real text — not a genuine forward elision)",
        not _quote_is_verbatim("logs the result ... validates input", ANSWER),
    )
    check(
        "empty quote fails",
        not _quote_is_verbatim("", ANSWER),
    )
    check(
        "ellipsis-only quote (no real fragments) fails",
        not _quote_is_verbatim("...", ANSWER),
    )

    # The exact real-world case that motivated this fix (Plan 23 follow-up,
    # 2026-09-28): a judge quote citing a real answer almost word-for-word,
    # with markdown and a middle elision, that the old check rejected.
    real_quote = _strip_md(
        "It recomputes `pending_by_item = required_qty − transferred_qty` "
        "from the Work Order's `required_items`, sums the qty being "
        "transferred per item ... and **throws an \"Excess Material "
        "Transfer\" error**"
    )
    real_answer = _strip_md(
        "It recomputes `pending_by_item = required_qty − transferred_qty` "
        "from the Work Order's `required_items`, sums the qty being "
        "transferred per item (keyed on item_code or original_item), and "
        "**throws an \"Excess Material Transfer\" error** if any item's "
        "transfer exceeds its pending qty."
    )
    check(
        "real Q05 case: elided quote with a genuine verbatim substance match passes",
        _quote_is_verbatim(real_quote, real_answer),
    )

    print("All tests passed.")


if __name__ == "__main__":
    main()
