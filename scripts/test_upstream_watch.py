"""Exercise refresh-ticket state transitions without reading or writing GitHub."""
import json
import subprocess
import unittest
from unittest.mock import patch

import check_upstream as watch


class IssueBoard:
    def __init__(self):
        self.issues = []
        self.comments = {}
        self.fail = None

    def read(self, argv, *, text):
        path = next(arg for arg in argv if arg.startswith("repos/")).split("?")[0]
        if path.endswith("/comments"):
            rows = self.comments.get(int(path.split("/")[-2]), [])
            operation = "comments"
        else:
            rows = self.issues
            operation = "issues"
        if self.fail == operation:
            raise subprocess.CalledProcessError(1, argv)
        if self.fail == "empty response":
            return ""
        # One row per page: a second-page report must not be mistaken for absence.
        pages = [[r] for r in rows] or [[]]
        return "\n".join(json.dumps(page) for page in (pages if "--paginate" in argv else pages[:1]))

    def write(self, argv, *, check):
        body = argv[argv.index("--body") + 1]
        if argv[1:3] == ["issue", "create"]:
            self.issues.append({"number": len(self.issues) + 1, "title": argv[argv.index("--title") + 1], "body": body})
        elif argv[1:3] == ["issue", "comment"]:
            self.comments.setdefault(int(argv[3]), []).append({"body": body})
        else:
            raise AssertionError(f"unexpected write: {argv}")

    def observe(self, tag):
        with (patch.dict(watch.TOOLS, {"zed": "zed-industries/zed"}, clear=True),
              patch.object(watch, "latest_release", return_value=tag),
              patch.object(watch, "supported_prefixes", return_value=["0."]),
              patch.object(watch.subprocess, "check_output", self.read),
              patch.object(watch.subprocess, "run", self.write)):
            watch.main()


class PackWatch(unittest.TestCase):
    def test_successive_tags_share_one_report_with_maintainer_edits(self):
        board = IssueBoard()
        board.issues.append({"number": 1, "title": "zed: tables lag upstream v1.2", "body": "", "pull_request": {}})
        board.observe("v1.2")
        issue = board.issues[1]
        issue["title"] = "Review syntax changes before widening support"
        issue["body"] += "\nMaintainer: keep the old configuration fixture."
        prior = dict(issue)
        board.observe("v1.2")
        board.observe("v1.20")
        board.comments[2].append({"body": "Maintainer: still investigating."})
        board.observe("v1.20")
        self.assertEqual(board.issues[1:], [prior])
        self.assertIn("/v1.2>", prior["body"])
        newer, note = board.comments[2]
        self.assertIn("/v1.20>", newer["body"])
        self.assertEqual(note["body"], "Maintainer: still investigating.")

    def test_retitled_legacy_ticket_is_adopted_without_replacing_review_scope(self):
        board = IssueBoard()
        issue = {"number": 248, "title": "Zed schema review", "body": (
            "Keep the keybinding review.\nUpstream zed-industries/zed released **v1.28.1**, "
            "but the `zed` data pack supports `['0.']`.\nDo not drop old fixtures.")}
        board.issues.append(dict(issue))
        board.observe("v1.28.1")
        self.assertFalse(board.comments)
        board.observe("v1.29")
        board.observe("v1.29")
        self.assertEqual(board.issues, [issue])
        comment, = board.comments[248]
        self.assertIn("/v1.29>", comment["body"])

    def test_failed_or_missing_lookup_never_authorizes_a_write(self):
        for failure in ("issues", "comments", "empty response"):
            with self.subTest(failure=failure):
                board = IssueBoard()
                if failure == "comments":
                    board.observe("v1.2")
                before = [dict(i) for i in board.issues]
                board.fail = failure
                with self.assertRaises((subprocess.CalledProcessError, ValueError)):
                    board.observe("v1.3")
                self.assertEqual(board.issues, before)
                self.assertFalse(board.comments)

    def test_existing_duplicates_are_left_for_triage(self):
        board = IssueBoard()
        board.issues = [{"number": n, "title": f"zed: tables lag upstream v{n}", "body": ""} for n in (1, 2)]
        with self.assertRaisesRegex(ValueError, "#1, #2"):
            board.observe("v3")
        self.assertEqual([i["number"] for i in board.issues], [1, 2])
        self.assertFalse(board.comments)
