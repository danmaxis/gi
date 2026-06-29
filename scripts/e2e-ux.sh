#!/usr/bin/env bash
#
# e2e-ux.sh — tmux-driven UX smoke tests for the gi line REPL + TUI.
#
# These drive the real raw-mode/alt-screen interface in a headless tmux pane and
# assert on the captured screen, so the box rendering, mode switching, resize
# behaviour, CJK width handling, and TUI layout are checked without a human.
# They exercise *rendering only* (no model turn), so no provider is required.
#
# Usage:  scripts/e2e-ux.sh            # uses ~/.local/bin/gi (or $GI_BIN)
# Requires: tmux. Run from a checkout root.
set -u

GI_BIN="${GI_BIN:-$HOME/.local/bin/gi}"
SESSION="gi_e2e_$$"
PASS=0
FAIL=0

if ! command -v tmux >/dev/null 2>&1; then
  echo "SKIP: tmux not available"
  exit 0
fi
if [ ! -x "$GI_BIN" ]; then
  echo "FAIL: gi binary not found/executable at $GI_BIN (set GI_BIN)"
  exit 1
fi

cleanup() { tmux kill-session -t "$SESSION" 2>/dev/null; }
trap cleanup EXIT

# start_pane <cols> <rows> [args...] — fresh gi session at a forced size.
start_pane() {
  local cols="$1" rows="$2"; shift 2
  tmux kill-session -t "$SESSION" 2>/dev/null
  # Pin the pane cwd to the repo so @-mention git discovery works regardless of
  # a stale tmux server cwd.
  tmux new-session -d -s "$SESSION" -x "$cols" -y "$rows" -c "$PWD"
  tmux set-window-option -t "$SESSION" window-size manual 2>/dev/null
  tmux resize-window -t "$SESSION" -x "$cols" -y "$rows" 2>/dev/null
  sleep 0.3
  tmux send-keys -t "$SESSION" "$GI_BIN $*" Enter
  sleep 2.5
}

resize_pane() { tmux resize-window -t "$SESSION" -x "$1" -y "$2" 2>/dev/null; sleep 0.4; }
send() { tmux send-keys -t "$SESSION" "$@"; sleep 0.4; }
capture() { tmux capture-pane -t "$SESSION" -p; }

# check <name> <condition-cmd...> — condition runs against stdin (the capture).
check() {
  local name="$1"; shift
  if "$@"; then echo "  PASS: $name"; PASS=$((PASS + 1));
  else echo "  FAIL: $name"; FAIL=$((FAIL + 1)); fi
}

# display width (CJK-aware) of the widest box-border line on screen.
box_width() {
  capture | grep -E '╭─|╰─' | python3 -c '
import sys,unicodedata
def w(s):
    return sum(2 if unicodedata.east_asian_width(c) in ("W","F") else (0 if unicodedata.combining(c) else 1) for c in s.rstrip())
print(max((w(l) for l in sys.stdin), default=0))'
}
count_on_screen() { capture | grep -c "$1"; }
capture_e() { tmux capture-pane -t "$SESSION" -e -p; }
# Submit a slash command in full-screen: type it, dismiss the `/` popup, send.
fs_slash() { send "$1"; send Escape; send Enter; sleep 0.6; }

echo "== gi UX e2e ($GI_BIN) =="

echo "[1] inline box fills the terminal width (80)"
start_pane 80 20 --inline
check "box width == 80" test "$(box_width)" -eq 80
check "default header shown" test "$(count_on_screen '· asks before edits')" -ge 1

echo "[2] Shift+Tab cycles modes in place (no stacking)"
send BTab; send BTab; send BTab   # default -> plan -> edit -> mugen
check "exactly one box header after cycling" test "$(capture | grep -cE '· (asks before edits|read-only|auto-accepts|無限)')" -eq 1
check "reached mugen (無限 in header)" test "$(count_on_screen '無限')" -ge 1
check "mugen CJK box still 80 wide (rectangular)" test "$(box_width)" -eq 80

echo "[3] resize narrow 80->50 leaves no remnant header"
send "resize check"
resize_pane 50 20
send "!"
check "still exactly one mugen header" test "$(count_on_screen 'mugen ·')" -eq 1
check "box re-fit to width 50" test "$(box_width)" -eq 50

echo "[4] resize wide 50->90 leaves no remnant header"
resize_pane 90 20
send "?"
check "still exactly one mugen header" test "$(count_on_screen 'mugen ·')" -eq 1
check "box re-fit to width 90" test "$(box_width)" -eq 90
send Escape  # clear pending input / abort

echo "[4b] Ctrl+O cycles the detail level in the status line"
start_pane 90 20 --inline
check "default status has no detail tag" test "$(capture | grep -c 'Ctrl+O)')" -eq 0
send C-o
check "Ctrl+O → verbose" test "$(count_on_screen 'verbose (Ctrl+O)')" -ge 1
send C-o
check "Ctrl+O → raw" test "$(count_on_screen 'raw (Ctrl+O)')" -ge 1
send C-o
check "Ctrl+O → back to compact (no tag)" test "$(capture | grep -c 'Ctrl+O)')" -eq 0

echo "[5] gi --tui renders the full-screen layout"
start_pane 90 24 --tui
check "transcript title 'gi ·' present" test "$(count_on_screen 'gi ·')" -ge 1
check "input box glyph ❯ present" test "$(count_on_screen '❯')" -ge 1
check "status bar (◈) present" test "$(count_on_screen '◈')" -ge 1
send "hello there"
check "typed text echoed in TUI input" test "$(count_on_screen 'hello there')" -ge 1
send Escape  # quit TUI
sleep 0.5
check "clean exit back to shell prompt" test "$(capture | grep -c '\$')" -ge 1

echo "[6] answer renders with '◂ gi' header + margin + separated paragraphs (needs a model)"
start_pane 90 30 --inline
tmux send-keys -t "$SESSION" 'In exactly two short paragraphs separated by a blank line, define recursion. Be brief.'
sleep 0.4; tmux send-keys -t "$SESSION" Enter
# Poll up to ~40s for the COMPLETED answer (the body renders once at block-stop,
# marked by the ✔ Done line; skip gracefully if no provider is configured).
answered=0
for _ in $(seq 1 20); do
  sleep 2
  if capture | grep -q '✔'; then answered=1; break; fi
  if capture | grep -qiE 'no model|not configured|unauthor'; then break; fi
done
if [ "$answered" -eq 1 ]; then
  check "answer shows the '◂ gi' header" test "$(count_on_screen '◂ gi')" -ge 1
  check "answer body is margined (4-space indent)" \
    bash -c "capture(){ tmux capture-pane -t '$SESSION' -p; }; capture | grep -qE '^    [A-Za-z]'"
  check "two paragraphs (a blank line inside the answer)" \
    bash -c "capture(){ tmux capture-pane -t '$SESSION' -p; }; [ \"\$(capture | grep -cE '^    [A-Za-z]')\" -ge 2 ]"
else
  echo "  SKIP: no model answered (provider not configured?) — answer-render checks skipped"
fi

echo "[7] boxed approval + session memory (a → no re-prompt) (needs a model)"
start_pane 90 30 --inline
tmux send-keys -t "$SESSION" 'Run the shell command: echo hello-from-gi'
sleep 0.4; tmux send-keys -t "$SESSION" Enter
boxed=0
for _ in $(seq 1 15); do
  sleep 2
  capture | grep -q 'approve · bash' && { boxed=1; break; }
  capture | grep -qiE 'no model|not configured|unauthor' && break
done
if [ "$boxed" -eq 1 ]; then
  check "approval is a box titled 'approve · bash'" test "$(count_on_screen 'approve · bash')" -ge 1
  check "box offers [y] [n] [a] [A] choices" \
    bash -c "tmux capture-pane -t '$SESSION' -p | grep -q '\[a\]lways this tool'"
  # Approve for the session, then trigger the same tool again — no new box.
  send "a"
  for _ in $(seq 1 8); do sleep 2; capture | grep -q 'hello-from-gi' && break; done
  tmux send-keys -t "$SESSION" 'Run the shell command: echo second-call'
  sleep 0.4; tmux send-keys -t "$SESSION" Enter
  for _ in $(seq 1 10); do sleep 2; capture | grep -q 'second-call' && break; done
  check "second same-tool call did NOT prompt again" \
    test "$(capture | grep -c 'approve · bash')" -le 1
else
  echo "  SKIP: model didn't call bash (provider not configured?) — approval checks skipped"
fi

echo "[8] TUI Ctrl+O re-renders captured tool output per detail level (needs a model)"
start_pane 90 30 --tui
tmux send-keys -t "$SESSION" 'Run the shell command: seq 1 40'
sleep 0.4; tmux send-keys -t "$SESSION" Enter
tooled=0
for _ in $(seq 1 18); do
  sleep 2
  capture | grep -q '⚙ bash' && { tooled=1; break; }
done
if [ "$tooled" -eq 1 ] && [ "$(count_on_screen 'Ctrl+O to expand')" -ge 1 ]; then
  # The transcript auto-sticks to the bottom, so the truncation hint is visible
  # without scrolling (no PageUp — that would scroll away from it).
  check "compact truncates tool output with a Ctrl+O hint" test "$(count_on_screen 'Ctrl+O to expand')" -ge 1
  send C-o   # → verbose
  check "Ctrl+O → verbose expands (no truncation hint)" test "$(count_on_screen 'Ctrl+O to expand')" -eq 0
  send Escape
elif [ "$tooled" -eq 1 ]; then
  echo "  SKIP: model's tool output too short to truncate — Ctrl+O checks skipped"
else
  echo "  SKIP: model didn't call bash in the TUI — TUI Ctrl+O checks skipped"
fi

# ── Full-screen mode (the default) ────────────────────────────────────────────

echo "[9] full-screen default: rounded boxes"
start_pane 100 24
check "rounded transcript title (╭ gi)" test "$(count_on_screen '╭ gi ·')" -ge 1
check "rounded input box (╭ message)" test "$(count_on_screen '╭ message')" -ge 1

echo "[10] slash popup filters as you type"
send "/he"
check "popup offers /help for '/he'" test "$(count_on_screen '/help')" -ge 1
send Escape; send BSpace BSpace BSpace

echo "[11] /help output renders inside the transcript"
fs_slash "/help"
# Output auto-sticks to the bottom, so assert the LAST entries are visible.
check "help output in transcript (/doctor visible)" test "$(count_on_screen '/doctor')" -ge 1
check "help 'Debug' section visible at bottom" test "$(count_on_screen 'Debug')" -ge 1

echo "[12] command output is colored (not monochrome)"
check "help output carries ANSI color" test "$(capture_e | grep -ac $'\x1b\\[')" -ge 1

echo "[13] /skills output renders inside the transcript"
start_pane 100 30
fs_slash "/skills"
check "skills report shown in transcript" test "$(count_on_screen 'Skills')" -ge 1

echo "[14] @-mention popup lists files"
start_pane 100 30
# Prefix-match rust/Cargo.* — the popup row carries '.toml'/'.lock' the input lacks.
send "look at @rust/Cargo"
check "@ popup offers a Cargo file" test "$(capture | grep -cE 'Cargo\.(toml|lock)')" -ge 1
send Escape

echo "[15] Shift+Tab cycles the mode in full-screen"
start_pane 100 24
send BTab
check "title switched to plan" test "$(count_on_screen 'gi · plan')" -ge 1

echo "[16] short terminal: newest output is visible (auto-stick to bottom)"
start_pane 70 12   # tiny viewport — overflowing output must scroll to the end
fs_slash "/help"
# /doctor is the last entry in /help; it must be on screen, not clipped above.
check "last help entry (/doctor) visible at the bottom" test "$(count_on_screen '/doctor')" -ge 1
check "top help section (REPL) is scrolled off" test "$(count_on_screen 'REPL')" -eq 0

echo "[16b] scrollbar appears only when scrolled up"
start_pane 100 16
fs_slash "/help"   # long output overflows the small viewport
check "no scrollbar while pinned to bottom" test "$(count_on_screen '█')" -eq 0
send PageUp; send PageUp
check "scrollbar thumb (█) appears after PageUp" test "$(count_on_screen '█')" -ge 1
# Scroll all the way up to reveal the very top of the output.
for _ in $(seq 1 20); do send PageUp; done
check "scrolling up reveals the top (REPL)" test "$(count_on_screen 'REPL')" -ge 1
for _ in $(seq 1 25); do send PageDown; done
check "scrollbar hides back at the bottom" test "$(count_on_screen '█')" -eq 0

# ── Model-gated full-screen cases (set GI_E2E_MODEL=1 to run) ──────────────────

if [ -n "${GI_E2E_MODEL:-}" ]; then
  echo "[17] streamed answer appears in the transcript"
  start_pane 100 30
  send "say hello in 3 words"; send Enter
  streamed=0
  for _ in $(seq 1 20); do sleep 2; capture | grep -q '◂ gi' && { streamed=1; break; }; done
  check "answer streamed (◂ gi present)" test "$streamed" -eq 1

  echo "[18] long answer auto-sticks to the bottom (newest line visible)"
  start_pane 100 20
  send "list the numbers 1 to 40, one per line"; send Enter
  done40=0
  for _ in $(seq 1 25); do sleep 2; capture | grep -qE '(^|[^0-9])40([^0-9]|$)' && { done40=1; break; }; done
  check "final line (40) visible at the bottom" test "$done40" -eq 1

  echo "[19] approval overlay appears and y runs the tool"
  start_pane 100 30
  send "create a file named e2e_fs.txt with the text hi using the write_file tool"; send Enter
  asked=0
  for _ in $(seq 1 20); do sleep 2; capture | grep -q 'approve ·' && { asked=1; break; }; done
  if [ "$asked" -eq 1 ]; then
    check "approval overlay shown" test "$(count_on_screen 'approve ·')" -ge 1
    send "y"
    check "overlay dismissed after y" test "$(count_on_screen 'approve ·')" -eq 0
  else
    echo "  SKIP: model didn't request approval"
  fi
else
  echo "[17-19] model-gated full-screen cases SKIPPED (set GI_E2E_MODEL=1)"
fi

echo "== e2e: $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ]
