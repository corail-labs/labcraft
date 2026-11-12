"""Tests of guard.py, run by `cargo xtask spec`. Each expected verdict comes from the rule of AGENTS.md (D-23,
Commits) and the decisions of the owner on the hook, not from the output of the hook. No file of the tests exists:
the hook judges names, and never opens them."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import guard

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

# The owner's list: the four extensions of D-23, and .vit, .ctt, .llb, .lvclass. Written out here rather than taken
# from guard.py, so that a name dropped there turns a test red.
EXTENSIONS = ["vi", "ctl", "lvproj", "lvlib", "vit", "ctt", "llb", "lvclass"]


def tool(name, **tool_input):
    return guard.judge({"tool_name": name, "tool_input": tool_input, "cwd": str(ROOT)})


def bash(command, cwd=ROOT):
    return guard.judge({"tool_name": "Bash", "tool_input": {"command": command}, "cwd": str(cwd)})


def powershell(command, cwd=ROOT):
    return guard.judge({"tool_name": "PowerShell", "tool_input": {"command": command}, "cwd": str(cwd)})


# The repository of LabCraft, written out rather than taken from guard.py, and a fork of it.
LABCRAFT_URL = "https://github.com/corail-labs/labcraft.git"
FORK_URL = "https://github.com/someone/labcraft.git"


class Repository:
    """A git repository in a temporary folder, on a branch of our choice, with no commit. Its remote `origin` is the
    repository of LabCraft, its remote `fork` a fork of it, unless `origin` says otherwise. Nothing is fetched or
    pushed: the hook only reads the configuration."""

    def __init__(self, branch, origin=LABCRAFT_URL):
        self.folder = tempfile.TemporaryDirectory()
        self.path = Path(self.folder.name)
        subprocess.run(["git", "init", "-q", str(self.path)], check=True)
        subprocess.run(["git", "-C", str(self.path), "symbolic-ref", "HEAD", "refs/heads/" + branch], check=True)
        subprocess.run(["git", "-C", str(self.path), "remote", "add", "origin", origin], check=True)
        subprocess.run(["git", "-C", str(self.path), "remote", "add", "fork", FORK_URL], check=True)

    def close(self):
        self.folder.cleanup()


class LabviewFilesThroughTools(unittest.TestCase):
    def test_every_extension_is_refused_by_read_whatever_its_case(self):
        for extension in EXTENSIONS:
            for name in ("Main." + extension, "MAIN." + extension.upper()):
                with self.subTest(name=name):
                    refusal = tool("Read", file_path="C:\\project\\" + name)
                    self.assertIsNotNone(refusal)
                    self.assertIn("D-23", refusal)

    def test_a_trailing_dot_or_space_that_windows_drops(self):
        self.assertIsNotNone(tool("Read", file_path="C:\\project\\Main.vi."))
        self.assertIsNotNone(tool("Read", file_path="C:\\project\\Main.ctl "))

    def test_grep_with_a_glob_or_a_path_is_refused(self):
        self.assertIsNotNone(tool("Grep", pattern="Wire", glob="*.vi"))
        self.assertIsNotNone(tool("Grep", pattern="Wire", glob="**/*.{vi,ctl}"))
        self.assertIsNotNone(tool("Grep", pattern="Wire", path="project/Main.lvlib"))

    def test_glob_with_a_pattern_is_refused(self):
        self.assertIsNotNone(tool("Glob", pattern="**/*.lvproj"))
        self.assertIsNotNone(tool("Glob", pattern="**/*.VI"))

    def test_the_other_tools_that_take_a_path_are_refused(self):
        self.assertIsNotNone(tool("Edit", file_path="a.ctl", old_string="x", new_string="y"))
        self.assertIsNotNone(tool("Write", file_path="a.vi", content="x"))
        self.assertIsNotNone(tool("NotebookEdit", notebook_path="a.llb", new_source="x"))
        self.assertIsNotNone(tool("LSP", operation="hover", filePath="a.vit", line=1, character=1))
        self.assertIsNotNone(tool("WebFetch", url="https://example.com/download/Main.vi", prompt="read it"))
        self.assertIsNotNone(tool("mcp__some_server__open_file", path="x/Main.ctt"))
        self.assertIsNotNone(tool("mcp__some_server__scan", options={"paths": ["a.lcf", "b.lvclass"]}))

    def test_other_files_and_text_pass(self):
        self.assertIsNone(tool("Read", file_path="scripts/plugin.vim"))
        self.assertIsNone(tool("Read", file_path="docs/vi/README.md"))
        self.assertIsNone(tool("Read", file_path="notes.vi.md"))
        self.assertIsNone(tool("Read", file_path="crates/ctl2/src/lib.rs"))
        self.assertIsNone(tool("Glob", pattern="**/*.lcf"))
        self.assertIsNone(tool("Glob", pattern="**/*.*"))
        # A glob that leaves the files out.
        self.assertIsNone(tool("Grep", pattern="Wire", glob="!*.vi"))
        # The pattern of Grep is a regex over text, not a path: the docs may be searched for the word.
        self.assertIsNone(tool("Grep", pattern=r"\.vi\b", path="AGENTS.md"))
        # The text of a file that is written may name an extension.
        self.assertIsNone(tool("Write", file_path="AGENTS.md", content="never read *.vi files"))
        self.assertIsNone(tool("Edit", file_path="AGENTS.md", old_string="a", new_string="*.lvproj"))
        self.assertIsNone(tool("Agent", description="review", prompt="check that guard.py refuses *.vi"))


class LabviewFilesThroughCommands(unittest.TestCase):
    def assertRefused(self, command, judge=bash):
        refusal = judge(command)
        self.assertIsNotNone(refusal, command)
        self.assertIn("D-23", refusal, command)

    def test_reading_a_file(self):
        self.assertRefused("cat project/Main.vi")
        self.assertRefused("head -c 100 'My Project/Main.ctl' | xxd")
        self.assertRefused("strings x.llb")
        self.assertRefused("type C:\\project\\Main.vi")
        self.assertRefused('Get-Content -Path "C:\\Users\\me\\project\\Main.lvproj"', powershell)
        self.assertRefused("gc .\\Main.vi -Encoding Byte", powershell)
        self.assertRefused("cat < Main.vi")
        self.assertRefused("echo ok && cat Main.vi")
        self.assertRefused("ls; cat Main.VI")

    def test_searching_for_files(self):
        self.assertRefused("find . -name '*.lvproj'")
        self.assertRefused('find . -iname "*.VI" -exec cat {} +')
        self.assertRefused("rg -g '*.vi' Wire")
        self.assertRefused("rg --type-add 'lv:*.vi' -tlv Wire")
        self.assertRefused("grep -r --include=*.ctl Wire .")
        self.assertRefused("ls **/*.lvlib")
        self.assertRefused("ls *.{vi,ctl}")
        self.assertRefused("Get-ChildItem -Recurse -Filter *.vi", powershell)
        self.assertRefused("Get-ChildItem -Recurse -Include *.lvclass | Select-String Wire", powershell)
        self.assertRefused("dir /s *.vit")
        # A regex names the extension too: `\.vi$` is a search for the files.
        self.assertRefused("rg --files | grep '\\.vi$'")
        self.assertRefused("rg --files | grep -E '\\.(vi|ctl)$'")
        # A wildcard in place of the extension.
        self.assertRefused("find . -name '*.lv*'")
        self.assertRefused("ls *.v?")
        self.assertRefused("ls *.[v]i")

    def test_python_and_other_interpreters(self):
        self.assertRefused("python -c \"print(open('Main.vi', 'rb').read())\"")
        self.assertRefused("python3 -c \"import pathlib; [p.read_bytes() for p in pathlib.Path('.').rglob('*.lvlib')]\"")
        self.assertRefused("node -e \"require('fs').readFileSync('a.ctl')\"")
        self.assertRefused("python - <<'EOF'\nprint(open('Main.vi').read())\nEOF")
        self.assertRefused("cat <<EOF | python -\nimport glob; print(glob.glob('*.vi'))\nEOF")

    def test_a_command_inside_a_command(self):
        self.assertRefused('bash -c "cat Main.vi"')
        self.assertRefused('sh -lc "find . -name *.ctl"')
        self.assertRefused('pwsh -Command "Get-Content Main.vi"')
        self.assertRefused('cmd /c "type Main.vi"')
        self.assertRefused("eval cat Main.vi")
        self.assertRefused("F=Main.vi; cat $F")
        self.assertRefused('echo "$(cat Main.vi)"')
        self.assertRefused('git commit -m "$(cat Main.vi)"')
        self.assertRefused('git commit -m "Notes: `cat Main.vi`"')
        self.assertRefused("echo $(cat Main.vi)")
        # Without quotes around the delimiter, the shell runs what a here-document substitutes.
        self.assertRefused("git commit -F - <<EOF\n$(cat Main.vi)\nEOF")

    def test_other_files_and_messages_pass(self):
        for command in [
            "vim plugin.vim",
            "vi notes.txt",
            "ls docs/vi/",
            "cat notes.vi.md",
            "cargo xtask spec",
            "git commit -m \"Hooks that refuse .vi, .ctl and .lvproj files\"",
            "git commit -am 'Refuse *.vi'",
            "git commit --message='Refuse *.vi'",
            'git commit -am"Refuse *.vi"',
            # With quotes around the delimiter, the body is only text.
            "git commit -F - <<'EOF'\nDo not run $(cat Main.vi)\nEOF",
            "ls *.*",
            "ls -a .c*",
            "ls ~/.v*",
            "rg 'foo.*bar' docs",
            "find . -name '*.md'",
            "git commit -m \"$(cat <<'EOF'\nHooks that refuse *.vi files\n\nCo-Authored-By: x\nEOF\n)\"",
            "git commit -F - <<'EOF'\nRefuse *.lvlib\nEOF",
            "gh pr create --title \"Refuse *.vi\" --body \"The hook refuses *.lvproj, *.ctl\"",
            "gh pr create --title x --body \"$(cat <<'EOF'\n- refuses *.vi\nEOF\n)\"",
        ]:
            with self.subTest(command=command):
                self.assertIsNone(bash(command))
        self.assertIsNone(powershell("git commit -m @'\nRefuse *.vi\n'@"))


class PushToMain(unittest.TestCase):
    def setUp(self):
        self.main = Repository("main")
        self.topic = Repository("hooks/guard")

    def tearDown(self):
        self.main.close()
        self.topic.close()

    def assertRefused(self, command, cwd=None, judge=bash):
        refusal = judge(command, cwd or self.topic.path)
        self.assertIsNotNone(refusal, command)
        self.assertIn("pushes to main", refusal, command)

    def test_a_refspec_that_writes_main(self):
        for command in [
            "git push origin main",
            "git push origin HEAD:main",
            "git push origin hooks/guard:main",
            "git push origin feature:refs/heads/main",
            "git push origin HEAD:refs/heads/main",
            "git push origin +main",
            "git push --force origin main",
            "git push -f origin main",
            "git push --force-with-lease origin HEAD:main",
            "git push origin :main",
            "git push origin --delete main",
            "git push -u origin main 2>&1",
            "git push origin 'refs/heads/*:refs/heads/*'",
            "git push --repo=origin main",
            "git push https://github.com/corail-labs/labcraft main",
            "git push git@github.com:Corail-Labs/LabCraft.git main",
            "git -C %s push origin main" % self.main.path.as_posix(),
            "git -c push.default=current push origin main",
            "/usr/bin/git push origin main",
            "git.exe push origin main",
            "cd %s && git push origin main" % self.main.path.as_posix(),
            'bash -c "git push origin main"',
            "echo ok; git push origin main",
            "if true; then git push origin main; fi",
            "! git push origin main",
            "while true; do git push origin main; done",
            "timeout 600 git push origin main",
            "timeout -s KILL 60 git push origin main",
            "git push origin $(git rev-parse HEAD):main",
            "git push origin `git rev-parse HEAD`:main",
        ]:
            with self.subTest(command=command):
                self.assertRefused(command)
        self.assertRefused("git push origin main", judge=powershell)

    def test_every_branch(self):
        self.assertRefused("git push --all origin")
        self.assertRefused("git push --branches origin")
        self.assertRefused("git push --mirror")

    def test_the_current_branch_when_it_is_main(self):
        self.assertRefused("git push", self.main.path)
        self.assertRefused("git push origin", self.main.path)
        self.assertRefused("git push -u origin HEAD", self.main.path)
        self.assertRefused("git push origin @", self.main.path)
        self.assertRefused("cd %s && git push" % self.main.path.as_posix(), self.topic.path)
        self.assertRefused("git -C %s push" % self.main.path.as_posix(), self.topic.path)

    def test_a_bare_push_to_an_upstream_main(self):
        # With push.default=upstream, `git push` writes the upstream of the branch, here main.
        subprocess.run(["git", "-C", str(self.topic.path), "config", "push.default", "upstream"], check=True)
        subprocess.run(["git", "-C", str(self.topic.path), "config", "branch.hooks/guard.merge", "refs/heads/main"], check=True)
        self.assertRefused("git push", self.topic.path)
        self.assertIsNone(bash("git push origin hooks/guard", self.topic.path))

    def test_the_main_of_a_fork_or_of_another_repository_passes(self):
        # A contributor may push to the main of their fork: only the repository of LabCraft is protected here.
        for command in ["git push fork main", "git push --force fork HEAD:main", "git push %s main" % FORK_URL]:
            with self.subTest(command=command):
                self.assertIsNone(bash(command, self.topic.path))
        fork = Repository("main", origin=FORK_URL)
        try:
            self.assertIsNone(bash("git push", fork.path))
            self.assertIsNone(bash("git push origin main", fork.path))
        finally:
            fork.close()
        # A remote that git cannot name: GitHub decides.
        self.assertIsNone(bash("git push nowhere main", self.topic.path))

    def test_a_topic_branch_passes(self):
        for command in [
            "git push origin hooks/guard",
            "git push -u origin hooks/guard",
            "git push --force-with-lease origin hooks/guard",
            "git push --force origin hooks/guard",
            "git push origin main:hooks/guard",
            "git push origin --delete old-topic",
            "git push origin HEAD",
            "git push",
            "git pull origin main",
            "git fetch origin main",
            "git log origin/main",
            "git rebase origin/main",
            "echo git push origin main",
        ]:
            with self.subTest(command=command):
                self.assertIsNone(bash(command, self.topic.path))
        # Tags only: the current branch is not pushed.
        self.assertIsNone(bash("git push origin --tags", self.main.path))


class MergeWithoutTheHead(unittest.TestCase):
    def setUp(self):
        self.clone = Repository("hooks/guard")

    def tearDown(self):
        self.clone.close()

    def assertRefused(self, command):
        refusal = bash(command, self.clone.path)
        self.assertIsNotNone(refusal, command)
        self.assertIn("labcraft-ship-step", refusal, command)

    def test_merge_without_match_head_commit(self):
        self.assertRefused("gh pr merge 12 --squash")
        self.assertRefused('gh pr merge 12 --squash --subject "x (#12)" --body "y"')
        self.assertRefused("gh pr merge --auto --squash")
        self.assertRefused("gh pr -R corail-labs/labcraft merge 12 --squash")
        self.assertRefused("timeout 600 gh pr merge 12 --squash")

    def test_merge_with_admin(self):
        self.assertRefused("gh pr merge 12 --squash --admin --match-head-commit 0123abc")
        self.assertRefused("gh pr merge 12 --squash --admin=true --match-head-commit 0123abc")

    def test_merge_through_the_api(self):
        self.assertRefused("gh api -X PUT repos/corail-labs/labcraft/pulls/12/merge -f merge_method=squash")
        self.assertRefused("gh api -X PUT repos/{owner}/{repo}/pulls/12/merge")
        self.assertRefused("gh api graphql -f query='mutation { mergePullRequest(input: {pullRequestId: \"x\"}) { clientMutationId } }'")
        self.assertRefused("gh api graphql -f query='mutation { enablePullRequestAutoMerge(input: {}) { clientMutationId } }'")
        self.assertRefused("gh api graphql -f query='mutation { enqueuePullRequest(input: {}) { clientMutationId } }'")
        self.assertRefused("gh api graphql -f query='mutation { mergeBranch(input: {}) { clientMutationId } }'")

    def test_writing_main_through_the_api(self):
        for command in [
            "gh api repos/corail-labs/labcraft/merges -f base=main -f head=topic",
            "gh api -X PATCH repos/corail-labs/labcraft/git/refs/heads/main -f sha=0123abc -F force=true",
            "gh api --method DELETE repos/corail-labs/labcraft/git/refs/heads/main",
            "gh api graphql -f query='mutation { updateRef(input: {refId: \"x\", oid: \"y\"}) { clientMutationId } }'",
        ]:
            with self.subTest(command=command):
                refusal = bash(command, self.clone.path)
                self.assertIsNotNone(refusal, command)
                self.assertIn("pushes to main", refusal, command)

    def test_what_passes(self):
        for command in [
            'gh pr merge 12 --squash --subject "x (#12)" --body "y" --match-head-commit 0123abc',
            "gh pr merge 12 --squash --match-head-commit=0123abc",
            "gh pr view 12",
            "gh pr checks 12",
            "gh api repos/corail-labs/labcraft/pulls/12",
            "gh api repos/corail-labs/labcraft/pulls/12/merges-count",
            "gh api repos/corail-labs/labcraft/git/refs/heads/main",
            "gh pr -R corail-labs/labcraft merge 12 --squash --match-head-commit 0123abc",
            # Another repository: the rule is ours only.
            "gh pr -R someone/labcraft merge 12 --squash",
            "gh api -X PUT repos/someone/labcraft/pulls/12/merge",
        ]:
            with self.subTest(command=command):
                self.assertIsNone(bash(command, self.clone.path))
        fork = Repository("hooks/guard", origin=FORK_URL)
        try:
            subprocess.run(["git", "-C", str(fork.path), "remote", "remove", "fork"], check=True)
            self.assertIsNone(bash("gh pr merge 12 --squash", fork.path))
        finally:
            fork.close()


class AsClaudeCodeRunsIt(unittest.TestCase):
    """The script itself, with JSON on standard input, as `.claude/settings.json` runs it."""

    def run_hook(self, stdin, command=None, env=None):
        command = command or [sys.executable, str(HERE / "guard.py")]
        return subprocess.run(command, input=stdin.encode("utf-8"), capture_output=True, env=env, timeout=30)

    def test_a_refusal_exits_2_and_tells_the_agent(self):
        result = self.run_hook(json.dumps({"tool_name": "Read", "tool_input": {"file_path": "Main.vi"}, "cwd": str(ROOT)}))
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"D-23", result.stderr)

    def test_a_call_that_passes_exits_0(self):
        result = self.run_hook(json.dumps({"tool_name": "Read", "tool_input": {"file_path": "AGENTS.md"}, "cwd": str(ROOT)}))
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, b"")

    def test_a_failure_of_the_hook_lets_the_call_through(self):
        # Exit 1 is a non-blocking error for Claude Code: the call goes on (fail open).
        result = self.run_hook("not json")
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"lets it through", result.stderr)

    def test_a_crafted_input_stays_fast(self):
        # Under a second, far from the timeout of the settings (15 s).
        for command in ["ls " + ".{" * 40000, "ls " + ".(" * 40000, "ls ." + "a" * 80000, "echo " + "$(" * 2000]:
            started = time.monotonic()
            guard.judge({"tool_name": "Bash", "tool_input": {"command": command}, "cwd": str(ROOT)})
            self.assertLess(time.monotonic() - started, 1.0, command[:20])

    def settings_command(self):
        settings = json.loads((ROOT / ".claude" / "settings.json").read_text(encoding="utf-8"))
        entries = settings["hooks"]["PreToolUse"]
        self.assertEqual([entry["matcher"] for entry in entries], ["*"])
        return entries[0]["hooks"][0]["command"]

    def test_the_settings_run_the_hook_for_every_tool_from_the_project(self):
        self.assertEqual(self.settings_command(), 'sh "$CLAUDE_PROJECT_DIR/.claude/hooks/guard.sh"')

    @unittest.skipIf(shutil.which("sh") is None, "no sh to run the command of the settings")
    def test_the_command_of_the_settings_finds_python_and_keeps_the_exit_code(self):
        # The path as it is: the wrapper must find a Python 3 by itself (python3 on Linux, python on Windows).
        env = dict(os.environ, CLAUDE_PROJECT_DIR=str(ROOT))
        sh = ["sh", "-c", self.settings_command()]
        refused = json.dumps({"tool_name": "Bash", "tool_input": {"command": "cat Main.vi"}, "cwd": str(ROOT)})
        passed = json.dumps({"tool_name": "Bash", "tool_input": {"command": "git status"}, "cwd": str(ROOT)})
        self.assertEqual(self.run_hook(refused, sh, env).returncode, 2)
        self.assertEqual(self.run_hook(passed, sh, env).returncode, 0)

    @unittest.skipIf(shutil.which("sh") is None, "no sh to run the wrapper")
    def test_without_python_the_wrapper_says_so_and_lets_the_call_through(self):
        env = dict(os.environ, PATH="")
        refused = json.dumps({"tool_name": "Read", "tool_input": {"file_path": "Main.vi"}, "cwd": str(ROOT)})
        result = self.run_hook(refused, [shutil.which("sh"), str(HERE / "guard.sh")], env)
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"no Python 3", result.stderr)


class TheIgnoreFile(unittest.TestCase):
    def test_it_lists_the_extensions_of_the_hook(self):
        lines = (ROOT / ".ignore").read_text(encoding="utf-8").splitlines()
        patterns = [line for line in lines if line and not line.startswith("#")]
        self.assertEqual(patterns, ["*." + extension for extension in EXTENSIONS])


if __name__ == "__main__":
    unittest.main()
