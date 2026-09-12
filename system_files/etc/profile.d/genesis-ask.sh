# Genesis in the terminal.
#   ask what you want          -> genesis-ask shows the command, runs it when you confirm
#   type a sentence, press Ctrl+G -> the line is replaced by the command (bash), then Enter runs it
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
fi
