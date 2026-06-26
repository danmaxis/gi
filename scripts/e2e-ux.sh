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
  tmux new-session -d -s "$SESSION" -x "$cols" -y "$rows"
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

echo "== gi UX e2e ($GI_BIN) =="

echo "[1] default box fills the terminal width (80)"
start_pane 80 20
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
start_pane 90 20
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
start_pane 90 30
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
start_pane 90 30
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

echo "== e2e: $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ]
