"""PreToolUse hook of Claude Code: two rules of AGENTS.md that an agent must not break by mistake.

1. D-23: no reading, searching or listing of LabVIEW files (`LABVIEW` below), whatever the tool.
2. Commits: never push to main; merge a pull request only with `gh pr merge --match-head-commit`, never with
   `--admin`, never through `gh api`. Only for the repository of LabCraft (`LABCRAFT`): a contributor may push to the
   main of their own fork, and cannot push to ours or merge anyway.

It judges the paths and the commands only. It never opens a file that a call names, and the only program it runs is
`git symbolic-ref`, to learn the current branch of a bare `git push`. A guard rail, not a sandbox: a script, a variable
or an alias gets past it.

Claude Code sends the call as JSON on standard input (`.claude/settings.json` runs `guard.sh`, which finds a Python 3).
Exit 2 refuses it and tells the agent why, on standard error; exit 0 lets it through. When the hook itself fails, it
exits 1, which Claude Code shows and lets through: fail open, the owner's choice. Tests: `test_guard.py`, run by
`cargo xtask spec`.
"""

import fnmatch
import json
import os
import re
import subprocess
import sys

# D-23 names the first four; the others are LabVIEW files as well. `.vim` (a malleable VI) is left out on purpose:
# it is also the extension of Vim scripts. `.ignore` at the root lists the same names, and a test keeps them in step.
LABVIEW = ("vi", "ctl", "lvproj", "lvlib", "vit", "ctt", "llb", "lvclass")

_EXTENSIONS = "|".join(LABVIEW)
# The extension ends the name: `.vim`, `.vi2`, `.vi-old` and `.vi.md` are other files; `*.vi"`, `a.vi;` and `x.vi/`
# are not.
_NAME = re.compile(r"\.(?:%s)(?![\w.\-])" % _EXTENSIONS, re.IGNORECASE)
# Alternatives in braces or parentheses: the glob `*.{vi,ctl}`, the regex `\.(vi|ctl)$`. `[^{}()]` stops at the next
# brace, so that the search stays linear.
_GROUP = re.compile(r"\.[{(]([^{}()]*)[})]")
# A wildcard in place of the extension: `*.lv*`, `*.v?`, `*.[v]i`. A dot that starts a name, `.c*` or `~/.v*`, is a
# hidden file, not an extension.
_WILD = re.compile(r"(?<=[^\s/\\'\"=])\.([\w\[\]?*!^-]*[*?\[][\w\[\]?*!^-]*)(?![.\-])")

# The keys of a tool's input that hold a path, a glob or an address. The pattern of Grep is a regex over the text of
# files, so it is not one; the pattern of Glob is.
_PATH_KEY = re.compile(r"path|file|dir|folder|glob|uri|url|root|cwd", re.IGNORECASE)

# Programs that run their argument as a command line.
_SHELLS = {"bash", "sh", "zsh", "dash", "ksh", "pwsh", "powershell", "cmd"}
# Words that come before the program they run, with their options.
_WRAPPERS = {"sudo", "env", "command", "builtin", "exec", "time", "nohup", "nice", "xargs", "&"}
# Words of the shell that come before a command.
_KEYWORDS = {"if", "then", "else", "elif", "while", "until", "do", "!"}
_CHANGE_DIRECTORY = {"cd", "chdir", "pushd", "set-location", "sl"}
_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")

# The subcommands of git that take a message, and the options of gh that carry text: a message may name an extension.
_GIT_MESSAGE_COMMANDS = {"commit", "tag", "notes", "merge", "revert", "stash"}
_GH_TEXT_GROUPS = {"pr", "issue", "release"}
_GH_TEXT_OPTIONS = {"-t", "--title", "-b", "--body", "--subject", "-n", "--notes"}

# The repository whose main rule 2 protects, as GitHub names it (case does not matter).
LABCRAFT = "corail-labs/labcraft"
_LABCRAFT = re.compile(r"(?:^|[/:])%s(?:\.git)?/?$" % re.escape(LABCRAFT), re.IGNORECASE)
_API_REPOSITORY = re.compile(r"repos/([^/\s{}]+/[^/\s{}]+)")

_GIT_OPTIONS_WITH_VALUE = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"}
_PUSH_OPTIONS_WITH_VALUE = {"-o", "--push-option", "--receive-pack", "--exec"}
_MERGE_API = re.compile(
    r"pulls/[^/\s]+/merge(?![\w-])|mergePullRequest|enablePullRequestAutoMerge|enqueuePullRequest|mergeBranch"
)
# Endpoints of the REST API that write a branch: a merge into a base, and a reference.
_MERGES_API = re.compile(r"repos/[^/\s]+/[^/\s]+/merges(?![\w-])")
_MAIN_REF_API = re.compile(r"git/refs/heads/main(?![\w/.\-])")
# The GraphQL mutations that move a reference, main among them.
_REF_MUTATION = re.compile(r"\bupdateRefs?\b")
_GH_API_FIELDS = {"-f", "-F", "--field", "--raw-field", "--input"}

# Escaped outside quotes. Any other backslash is kept: on Windows it separates the parts of a path.
_ESCAPABLE = " \t'\"\\;&|()<>`$"
_HEREDOC = re.compile(r"<<(-?)[ \t]*(['\"]?)([A-Za-z0-9_.\-]+)\2")


def _extension_pattern(pattern):
    """Does a pattern in place of an extension match a LabVIEW one? `*` alone matches every file, so it does not."""
    pattern = pattern.strip().lower()
    if not pattern or set(pattern) <= {"*"}:
        return False
    return any(fnmatch.fnmatchcase(extension, pattern) for extension in LABVIEW)


def _labview(text):
    """The part of `text` that names a LabVIEW file, or None."""
    # Windows drops the dots and spaces that end a name: `Main.vi.` is `Main.vi`.
    for candidate in (text, text.rstrip(". ")):
        match = _NAME.search(candidate)
        if match:
            return match.group(0)
    for match in _GROUP.finditer(text):
        if any(_extension_pattern(part) for part in re.split(r"[,|]", match.group(1))):
            return match.group(0)
    for match in _WILD.finditer(text):
        if _extension_pattern(match.group(1)):
            return match.group(0)
    return None


def _refuse_labview(what):
    return (
        "Refused by .claude/hooks/guard.py (D-23 in AGENTS.md): `%s` names a LabVIEW file (%s). Never read, search "
        "or list LabVIEW files for this project, and do not look for another way to do it. If the extension is only "
        "text (a word of a message, a regex over the docs), reword it, write the file with the Write tool, or search "
        "with the Grep tool, whose pattern is not judged." % (_short(what), ", ".join("." + e for e in LABVIEW))
    )


def _refuse_push(command, reason):
    return (
        "Refused by .claude/hooks/guard.py (AGENTS.md, Commits): `%s` pushes to main (%s). main is protected: every "
        "change goes through a topic branch and a pull request. Push the topic branch." % (_short(command), reason)
    )


def _refuse_merge(command, reason):
    return "Refused by .claude/hooks/guard.py (AGENTS.md, Commits; skill labcraft-ship-step, stage 5): `%s`: %s" % (
        _short(command),
        reason,
    )


def _short(text):
    text = " ".join(text.split())
    return text if len(text) <= 120 else text[:117] + "..."


class _Segment:
    """One simple command: its words, and the bodies of its here-documents, each with whether the shell expands the
    commands in it (a delimiter without quotes)."""

    def __init__(self):
        self.words = []
        self.bodies = []


def _closing(text, i):
    """The index of the `)` that closes a `(` opened just before `i`, or the length of `text`."""
    depth = 1
    quote = None
    while i < len(text):
        c = text[i]
        if quote:
            if c == quote:
                quote = None
        elif c in "'\"":
            quote = c
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(text)


def _substitutions(text):
    """The commands that the shell runs inside `text`: each `$( … )` and each `` ` … ` ``."""
    found = []
    i = 0
    while i < len(text):
        if text.startswith("$(", i):
            end = _closing(text, i + 2)
            found.append(text[i + 2 : end])
            i = end + 1
        elif text[i] == "`":
            end = text.find("`", i + 1)
            end = len(text) if end < 0 else end
            found.append(text[i + 1 : end])
            i = end + 1
        else:
            i += 1
    return found


def _segments(command, depth=0):
    """Splits a command line into simple commands, roughly as a POSIX shell or PowerShell would. A `$( … )` or a
    `` ` … ` `` outside quotes stays in its word, and its commands are commands of their own. Never fails."""
    segments = []
    nested = []
    current = _Segment()
    token = []
    started = False
    quote = None
    pending = []  # here-documents whose body starts at the next new line: (delimiter, strip tabs, expands, segment)
    i = 0
    n = len(command)

    def end_token():
        nonlocal token, started
        if started:
            current.words.append("".join(token))
        token = []
        started = False

    def end_segment():
        nonlocal current
        end_token()
        if current.words or current.bodies:
            segments.append(current)
        current = _Segment()

    while i < n:
        c = command[i]
        if quote == "'":
            if c == "'":
                quote = None
            else:
                token.append(c)
            i += 1
            continue
        if quote == '"':
            if c == '"':
                quote = None
            elif c == "\\" and i + 1 < n and command[i + 1] in '"\\$`':
                token.append(command[i + 1])
                i += 1
            else:
                token.append(c)
            i += 1
            continue
        if c in "'\"":
            quote = c
            started = True
        elif c == "\\" and i + 1 < n:
            following = command[i + 1]
            if following == "\n":
                i += 1
            elif following in _ESCAPABLE:
                token.append(following)
                started = True
                i += 1
            else:
                token.append(c)
                started = True
        elif c in " \t\r":
            end_token()
        elif c == "\n":
            end_segment()
            if pending:
                i = _read_bodies(command, i + 1, pending)
                pending = []
                continue
        elif (c == "$" and command.startswith("$(", i)) or c == "`":
            end = _closing(command, i + 2) if c == "$" else command.find("`", i + 1)
            end = n if end < 0 else end
            inner = command[i + 2 : end] if c == "$" else command[i + 1 : end]
            token.append(command[i : end + 1])
            started = True
            if depth < 8:
                nested.extend(_segments(inner, depth + 1))
            i = end + 1
            continue
        elif c in ";|&()":
            end_segment()
        elif c in "<>":
            end_token()
            heredoc = _HEREDOC.match(command, i) if command.startswith("<<", i) and not command.startswith("<<<", i) else None
            if heredoc:
                current.words.append("<<")
                current.words.append(heredoc.group(3))
                pending.append((heredoc.group(3), heredoc.group(1) == "-", not heredoc.group(2), current))
                i = heredoc.end()
                continue
            j = i
            while j < n and command[j] in "<>&":
                j += 1
            current.words.append(command[i:j])
            i = j
            continue
        else:
            token.append(c)
            started = True
        i += 1
    end_segment()
    return [part for segment in segments for part in _split_braces(segment)] + nested


def _read_bodies(command, i, pending):
    """Reads the bodies of the pending here-documents from position `i`; returns the position after the last one."""
    for delimiter, strip_tabs, expands, segment in pending:
        lines = []
        while i < len(command):
            end = command.find("\n", i)
            line = command[i:] if end < 0 else command[i:end]
            i = len(command) if end < 0 else end + 1
            if (line.lstrip("\t") if strip_tabs else line).rstrip("\r") == delimiter:
                break
            lines.append(line)
        segment.bodies.append(("\n".join(lines), expands))
    return i


def _split_braces(segment):
    """A `{` or `}` that stands alone opens or closes a block: the command after it is a command of its own."""
    parts = [_Segment()]
    for word in segment.words:
        if word in ("{", "}"):
            parts.append(_Segment())
        else:
            parts[-1].words.append(word)
    parts[0].bodies = segment.bodies
    return [part for part in parts if part.words or part.bodies]


def _name(word):
    name = word.replace("\\", "/").rsplit("/", 1)[-1].lower()
    return name[:-4] if name.endswith(".exe") else name


def _program(words):
    """The name of the program a simple command runs, and its index; (None, len) when there is none."""
    i = 0
    while i < len(words):
        word = words[i]
        if _ASSIGNMENT.match(word):
            i += 1
            continue
        name = _name(word)
        if name in _KEYWORDS:
            i += 1
            continue
        if name == "timeout":  # its options, then the duration
            i += 1
            while i < len(words) and words[i].startswith("-"):
                i += 2 if words[i] in ("-s", "-k", "--signal", "--kill-after") else 1
            i += 1
            continue
        if name in _WRAPPERS:
            i += 1
            while i < len(words) and words[i].startswith("-"):
                i += 1
            continue
        return name, i
    return None, len(words)


def _inner_command(name, words, start):
    """For `bash -c TEXT`, `pwsh -Command TEXT`, `cmd /c TEXT` and `eval TEXT`: the indices of TEXT and TEXT."""
    if name == "eval":
        rest = list(range(start + 1, len(words)))
        return rest, " ".join(words[start + 1 :])
    if name not in _SHELLS:
        return [], None
    for j in range(start + 1, len(words) - 1):
        word = words[j].lower()
        if word in ("-command", "/c", "/k") or re.match(r"^-[a-z]*c$", word):
            return [j + 1], words[j + 1]
    return [], None


def _git_subcommand(words, start):
    """The index of the subcommand of git, after its global options."""
    i = start + 1
    while i < len(words):
        word = words[i]
        if word in _GIT_OPTIONS_WITH_VALUE:
            i += 2
        elif word.startswith("-"):
            i += 1
        else:
            return i
    return None


def _gh_group_action(words, start):
    """The first two words of gh that are not options: `pr merge`, `api repos/...`. `-R` and `--repo` take a value."""
    positional = []
    i = start + 1
    while i < len(words) and len(positional) < 2:
        word = words[i]
        if word in ("-R", "--repo"):
            i += 1
        elif not word.startswith("-"):
            positional.append(word)
        i += 1
    return (positional + [None, None])[:2]


def _message_indices(name, words, start):
    """The indices of the words that are the text of a message, not a path."""
    if name == "git":
        sub = _git_subcommand(words, start)
        if sub is None or words[sub] not in _GIT_MESSAGE_COMMANDS:
            return set()
        indices = set()
        for j in range(sub + 1, len(words)):
            word = words[j]
            # `-m`, `-am`: the message is the next word; `-mText`, `-amText`: it is in this one.
            short = None if word.startswith("--") else re.match(r"^-[a-zA-Z]*m", word)
            if word == "--message" or (short and short.end() == len(word)):
                indices.add(j + 1)
            elif word.startswith("--message=") or short:
                indices.add(j)
        return indices
    if name == "gh" and _gh_group_action(words, start)[0] in _GH_TEXT_GROUPS:
        indices = set()
        for j in range(start + 1, len(words)):
            word = words[j]
            if word in _GH_TEXT_OPTIONS:
                indices.add(j + 1)
            elif any(word.startswith(option + "=") for option in _GH_TEXT_OPTIONS if option.startswith("--")):
                indices.add(j)
        return indices
    return set()


def _takes_message(name, words, start):
    """Does this command read its message from a here-document?"""
    if name == "git":
        sub = _git_subcommand(words, start)
        return sub is not None and words[sub] in _GIT_MESSAGE_COMMANDS
    return name == "gh" and _gh_group_action(words, start)[0] in _GH_TEXT_GROUPS


def _windows_path(path):
    """`/d/ideas` of Git Bash is `d:/ideas` for Windows."""
    match = re.match(r"^/([A-Za-z])(/|$)", path)
    return match.group(1) + ":/" + path[3:] if os.name == "nt" and match else path


def _directory(base, path):
    return os.path.join(base, os.path.expanduser(_windows_path(path)))


def _git(directory, *arguments):
    """What `git -C directory ARGUMENTS` prints, or None when it fails (no repository, no such value, no git)."""
    try:
        result = subprocess.run(["git", "-C", directory, *arguments], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip() or None


def _current_branch(directory):
    """The current branch in `directory`, or None when git cannot tell (no repository, detached head, no git)."""
    return _git(directory, "symbolic-ref", "--short", "-q", "HEAD")


def _bare_push_target(directory):
    """The branch that `git push` with no refspec writes: the current one, or its upstream when `push.default` says
    so. With the default, `simple`, git itself refuses a push to an upstream of another name."""
    branch = _current_branch(directory)
    if branch and (_git(directory, "config", "--get", "push.default") or "").lower() in ("upstream", "tracking"):
        upstream = _git(directory, "config", "--get", "branch.%s.merge" % branch) or ""
        return upstream[len("refs/heads/") :] if upstream.startswith("refs/heads/") else upstream or branch
    return branch


def _is_labcraft(address):
    """Is a remote URL, or an `owner/name` of gh, the repository of LabCraft?"""
    return bool(address and _LABCRAFT.search(address.strip()))


def _remote_address(directory, remote):
    """The URL a push to `remote` goes to: `remote` itself when it is a URL or a path, else its configuration."""
    if re.search(r"[:/\\]", remote):
        return remote
    return _git(directory, "config", "--get", "remote.%s.pushurl" % remote) or _git(
        directory, "config", "--get", "remote.%s.url" % remote
    )


def _default_remote(directory):
    """The remote of a `git push` that names none, in the order git looks for it."""
    branch = _current_branch(directory)
    keys = ["branch.%s.pushRemote" % branch] if branch else []
    keys.append("remote.pushDefault")
    keys += ["branch.%s.remote" % branch] if branch else []
    for key in keys:
        remote = _git(directory, "config", "--get", key)
        if remote:
            return remote
    return "origin"


def _has_labcraft_remote(directory):
    """Is one of the remotes of the repository in `directory` the repository of LabCraft? gh picks among them."""
    lines = (_git(directory, "remote", "-v") or "").splitlines()
    return any(_is_labcraft(line.split()[1]) for line in lines if len(line.split()) >= 2)


def _destination(refspec, directory):
    """The branch a refspec writes on the remote: `None` when it names no branch, `*` for a pattern."""
    if refspec.startswith("^"):
        return None
    refspec = refspec.lstrip("+")
    source, colon, target = refspec.partition(":")
    target = target if colon and target else source
    if target in ("HEAD", "@"):
        target = _current_branch(directory) or ""
    for prefix in ("refs/heads/", "heads/"):
        if target.startswith(prefix):
            target = target[len(prefix) :]
    return "*" if "*" in target else target or None


def _judge_push(command, words, directory):
    positional = []
    repository_option = None
    tags = False
    every = None
    options_ended = False
    i = 0
    while i < len(words):
        word = words[i]
        if word.startswith(("<", ">")):
            i += 2
            continue
        if options_ended:
            positional.append(word)
        elif word == "--":
            options_ended = True
        elif word in ("--all", "--branches", "--mirror"):
            every = word
        elif word == "--repo":
            repository_option = words[i + 1] if i + 1 < len(words) else ""
            i += 1
        elif word.startswith("--repo="):
            repository_option = word.split("=", 1)[1]
        elif word in _PUSH_OPTIONS_WITH_VALUE:
            i += 1
        elif word == "--tags":
            tags = True
        elif not word.startswith("-"):
            positional.append(word)
        i += 1
    if repository_option is not None:
        remote, refspecs = repository_option, positional
    else:
        remote, refspecs = (positional[0] if positional else _default_remote(directory)), positional[1:]
    if not _is_labcraft(_remote_address(directory, remote)):
        return None  # a fork, another repository, or a remote git cannot name: GitHub decides
    if every:
        return _refuse_push(command, every + " pushes every branch, main too")
    if not refspecs:
        if not tags and _bare_push_target(directory) == "main":
            return _refuse_push(command, "the current branch, or the upstream it pushes to, is main")
        return None
    for refspec in refspecs:
        target = _destination(refspec, directory)
        if target == "main":
            return _refuse_push(command, "`%s` writes main" % refspec)
        if target == "*":
            return _refuse_push(command, "the pattern `%s` can write main" % refspec)
    return None


def _judge_git(command, words, start, directory):
    i = start + 1
    while i < len(words):
        word = words[i]
        if word == "-C" and i + 1 < len(words):
            directory = _directory(directory, words[i + 1])
            i += 2
        elif word in _GIT_OPTIONS_WITH_VALUE:
            i += 2
        elif word.startswith("-"):
            i += 1
        else:
            break
    if i < len(words) and words[i] == "push":
        return _judge_push(command, words[i + 1 :], directory)
    return None


def _gh_targets_labcraft(words, directory):
    """Does a gh command act on the repository of LabCraft? `-R` names it, or the path of `gh api`; otherwise gh
    takes one of the remotes of the current repository."""
    for i, word in enumerate(words):
        if word in ("-R", "--repo") and i + 1 < len(words):
            return _is_labcraft(words[i + 1])
        if word.startswith("--repo="):
            return _is_labcraft(word.split("=", 1)[1])
    for word in words:
        match = _API_REPOSITORY.search(word)
        if match:
            return _is_labcraft(match.group(1))
    return _has_labcraft_remote(directory)


def _judge_gh(command, words, start, directory):
    group, action = _gh_group_action(words, start)
    rest = words[start + 1 :]
    if group not in ("pr", "api") or not _gh_targets_labcraft(rest, directory):
        return None
    if group == "pr" and action == "merge":
        if any(word == "--admin" or word.startswith("--admin=") for word in rest):
            return _refuse_merge(
                command,
                "--admin bypasses the protection of main. If a permission check refuses the merge, say so and leave "
                "the choice to the owner.",
            )
        if not any(word == "--match-head-commit" or word.startswith("--match-head-commit=") for word in rest):
            return _refuse_merge(
                command,
                "gh pr merge needs --match-head-commit with the head commit whose checks are green, and merges only "
                "on the owner's word for this pull request.",
            )
    if group != "api":
        return None
    if any(_MERGE_API.search(word) for word in rest):
        return _refuse_merge(
            command, "merging through gh api bypasses the rules of gh pr merge; use gh pr merge --match-head-commit."
        )
    if any(_REF_MUTATION.search(word) for word in rest):
        return _refuse_push(command, "a GraphQL mutation that moves a reference can move main")
    if any(_MERGES_API.search(word) for word in rest):
        return _refuse_push(command, "the merges endpoint writes a base branch")
    if any(_MAIN_REF_API.search(word) for word in rest) and _gh_api_writes(rest):
        return _refuse_push(command, "the API writes the reference of main")
    return None


def _gh_api_writes(words):
    """Does a `gh api` call send something other than GET? Fields make it a POST."""
    for i, word in enumerate(words):
        method = words[i + 1] if word in ("-X", "--method") and i + 1 < len(words) else None
        method = word.split("=", 1)[1] if word.startswith("--method=") else method
        if method and method.upper() != "GET":
            return True
        if word in _GH_API_FIELDS or any(word.startswith(option + "=") for option in _GH_API_FIELDS if option[1] == "-"):
            return True
    return False


def judge_command(command, cwd, depth=0, in_message=False):
    """The refusal of a command line (Bash, PowerShell), or None."""
    directory = cwd
    for segment in _segments(command):
        words = segment.words
        name, start = _program(words)
        skipped, inner = _inner_command(name, words, start) if name else ([], None)
        if inner is not None and depth < 4:
            refusal = judge_command(inner, directory, depth + 1)
            if refusal:
                return refusal
        messages = _message_indices(name, words, start) if name else set()
        for j, word in enumerate(words):
            if j in skipped:
                continue
            if j in messages:
                # The text of the message is free, but the shell runs what is substituted in it.
                for inside in _substitutions(word) if depth < 4 else ():
                    refusal = judge_command(inside, directory, depth + 1, in_message=True)
                    if refusal:
                        return refusal
                continue
            if _labview(word):
                return _refuse_labview(word)
        message = in_message or (name and _takes_message(name, words, start))
        for body, expands in segment.bodies:
            if not message:
                found = _labview(body)
                if found:
                    return _refuse_labview(found)
            elif expands and depth < 4:
                # The body of a message is free, but without quotes around its delimiter the shell runs what is
                # substituted in it.
                for inside in _substitutions(body):
                    refusal = judge_command(inside, directory, depth + 1)
                    if refusal:
                        return refusal
        if name in _CHANGE_DIRECTORY:
            arguments = [word for word in words[start + 1 :] if not word.startswith("-")]
            directory = _directory(directory, arguments[-1]) if arguments else os.path.expanduser("~")
        elif name == "git":
            refusal = _judge_git(command, words, start, directory)
            if refusal:
                return refusal
        elif name == "gh":
            refusal = _judge_gh(command, words, start, directory)
            if refusal:
                return refusal
    return None


def _leaves(value, key=None):
    """Every string of a tool's input, with the key of the object that holds it."""
    if isinstance(value, dict):
        for child_key, child in value.items():
            yield from _leaves(child, str(child_key))
    elif isinstance(value, list):
        for child in value:
            yield from _leaves(child, key)
    elif isinstance(value, str):
        yield key, value


def judge(call):
    """The refusal of a tool call, as Claude Code sends it to a PreToolUse hook, or None."""
    tool = call.get("tool_name") or ""
    cwd = call.get("cwd") or os.getcwd()
    for key, value in _leaves(call.get("tool_input") or {}):
        if key == "command":
            refusal = judge_command(value, cwd)
            if refusal:
                return refusal
        elif key and (_PATH_KEY.search(key) or (tool == "Glob" and key == "pattern")) and _labview(value):
            if "glob" in key.lower() and value.startswith("!"):
                continue  # a glob that leaves these files out
            return _refuse_labview(value)
    return None


def main():
    try:
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass
    try:
        call = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        refusal = judge(call) if isinstance(call, dict) else None
    except Exception as error:  # fail open: a broken hook must not stop the session
        sys.stderr.write("guard.py could not judge this call (%s: %s); it lets it through\n" % (type(error).__name__, error))
        return 1
    if refusal:
        sys.stderr.write(refusal + "\n")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
