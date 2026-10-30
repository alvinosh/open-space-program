import unittest

import check_commit as c


class Message(unittest.TestCase):
    def ok(self, message):
        scopes, errors = c.check_message(message)
        self.assertEqual(errors, [], message)
        return scopes

    def bad(self, message, needle):
        _, errors = c.check_message(message)
        self.assertTrue(any(needle in e for e in errors), (message, errors))

    def test_accepts(self):
        self.assertEqual(self.ok("feat(sim): add hot staging"), ["sim"])
        self.assertEqual(self.ok("fix(sim,game): stop the plume swinging\n\nBody text."), ["sim", "game"])
        self.assertEqual(self.ok("feat(ui)!: replace the window layout format"), ["ui"])

    def test_skips_merges_and_reverts(self):
        self.assertEqual(c.check_message("Merge branch 'x' into main"), (None, []))
        self.assertEqual(c.check_message('Revert "feat(sim): add hot staging"'), (None, []))

    def test_fixup_only_when_allowed(self):
        self.assertEqual(c.check_message("fixup! feat(sim): add hot staging", allow_fixup=True), (None, []))
        self.assertNotEqual(c.check_message("fixup! feat(sim): add hot staging")[1], [])

    def test_rejects(self):
        self.bad("Staging", "must look like")
        self.bad("feat: add hot staging", "must look like")
        self.bad("feat(): add hot staging", "at least one scope")
        self.bad("wip(sim): add hot staging", "unknown type")
        self.bad("feat(physics): add hot staging", "unknown scope")
        self.bad("feat(sim,sim): add hot staging", "twice")
        self.bad("feat(sim): add hot staging.", "full stop")
        self.bad("feat(sim): Add hot staging", "lower-case")
        self.bad("feat(sim): fix", "too short")
        self.bad("feat(sim): " + "x" * 70, "limit")
        self.bad("feat(sim): add hot staging\nbody without a blank line", "blank line")


class Files(unittest.TestCase):
    def test_scope_of(self):
        self.assertEqual(c.scope_of("crates/osp_sim/src/staging.rs"), "sim")
        self.assertEqual(c.scope_of("README.md"), "docs")
        self.assertEqual(c.scope_of("docs/modding.md"), "docs")
        self.assertEqual(c.scope_of(".tasks/T001-x/task.json"), "tasks")
        self.assertEqual(c.scope_of(".githooks/commit-msg"), "repo")
        self.assertEqual(c.scope_of("Cargo.lock"), "incidental")
        self.assertIsNone(c.scope_of("something/new.txt"))

    def test_matching_scopes_pass(self):
        files = ["crates/osp_sim/src/a.rs", "crates/osp_game/src/b.rs", "Cargo.lock"]
        self.assertEqual(c.check_files(["sim", "game"], files), [])

    def test_unnamed_scope_is_rejected(self):
        errors = c.check_files(["sim"], ["crates/osp_sim/src/a.rs", "crates/osp_game/src/b.rs"])
        self.assertEqual(len(errors), 1)
        self.assertIn("scope `game`", errors[0])

    def test_scope_without_files_is_rejected(self):
        errors = c.check_files(["sim", "ui"], ["crates/osp_sim/src/a.rs"])
        self.assertEqual(len(errors), 1)
        self.assertIn("scope `ui` is named", errors[0])

    def test_unmapped_file_is_rejected(self):
        self.assertEqual(len(c.check_files(["sim"], ["elsewhere/x.rs"])), 1)

    def test_only_incidental_files_pass_any_scope(self):
        self.assertEqual(c.check_files(["build"], ["Cargo.lock"]), [])


if __name__ == "__main__":
    unittest.main()
