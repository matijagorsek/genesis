# Genesis in the terminal.
#   ask what you want          -> genesis-ask shows the command, runs it when you confirm
#   type a sentence, press Ctrl+G -> the line is replaced by the command (bash), then Enter runs it
#   a command fails             -> one dim line offers Ctrl+G to explain and fix it
ask() { genesis-ask "$@"; }
if [ -n "$BASH_VERSION" ] && [[ $- == *i* ]]; then
  __genesis_ask_line() {
    local q="$READLINE_LINE"
    [ -n "$q" ] || return 0
    local cmd
    cmd="$(genesis-ask --print "$q" 2>/dev/null)" || return 0
    [ -n "$cmd" ] && { READLINE_LINE="$cmd"; READLINE_POINT=${#cmd}; }
  }
  bind -x '"\C-g": __genesis_ask_line' 2>/dev/null

  # Terminal rescue: after a command fails, the prompt says so once and Ctrl+G then explains the failure
  # and offers a fix instead of turning your last sentence into a command.
  __genesis_last_failed=""
  __genesis_after_command() {
    local rc=$?
    local last
    last="$(fc -ln -1 2>/dev/null | sed 's/^[[:space:]]*//')"
    if [ "$rc" -ne 0 ] && [ -n "$last" ] && [ "$rc" -ne 130 ]; then
      __genesis_last_failed="$last"
      printf '\033[2m  that failed (exit %s) · Ctrl+G: explain and fix\033[0m\n' "$rc"
    else
      __genesis_last_failed=""
    fi
    return 0
  }
  case "$PROMPT_COMMAND" in *__genesis_after_command*) ;; *) PROMPT_COMMAND="__genesis_after_command${PROMPT_COMMAND:+; $PROMPT_COMMAND}";; esac

  __genesis_rescue_or_ask() {
    if [ -z "$READLINE_LINE" ] && [ -n "$__genesis_last_failed" ]; then
      local out
      out="$(genesis-ask --fix "$__genesis_last_failed" 2>/dev/null)" || return 0
      [ -n "$out" ] && { READLINE_LINE="$out"; READLINE_POINT=${#out}; }
      return 0
    fi
    __genesis_ask_line
  }
  bind -x '"\C-g": __genesis_rescue_or_ask' 2>/dev/null
fi
