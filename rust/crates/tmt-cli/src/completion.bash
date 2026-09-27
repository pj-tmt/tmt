
# Dynamic identity discovery uses TMT's grammar; command arguments belong to the
# selected command's completion, never to a second provider list here.
_tmt() {
    local line offset spec function_name command_name quoted before open_quote
    local -a result query_words
    local query_cword=$COMP_CWORD
    query_words=("${COMP_WORDS[@]}")
    if declare -F _get_comp_words_by_ref >/dev/null; then
        _get_comp_words_by_ref -n = -w query_words -i query_cword
    elif ((COMP_CWORD >= 2)) && [[ ${COMP_WORDS[COMP_CWORD-1]} == = && ${COMP_WORDS[COMP_CWORD-2]} == --identity ]]; then
        # Readline splits '=' even inside an option word. Only rejoin this
        # tokenization when the actual input has no separating whitespace.
        before=${COMP_LINE:0:COMP_POINT-${#COMP_WORDS[COMP_CWORD]}}
        if [[ $before == *--identity= ]]; then
            query_words=("${COMP_WORDS[@]:0:COMP_CWORD-2}" "--identity=${COMP_WORDS[COMP_CWORD]}")
            query_cword=$((COMP_CWORD-2))
        fi
    fi
    # Readline can retain an unfinished opening quote in the current token.
    # Remove only that prefix for identity lookup; earlier arguments stay exact.
    case ${query_words[query_cword]} in
        \"*|\'*)
            open_quote=${query_words[query_cword]:0:1}
            query_words[query_cword]=${query_words[query_cword]:1}
            ;;
    esac
    result=()
    while IFS= read -r line; do result+=("$line"); done < <(
        "${query_words[0]}" __complete -- "${query_words[@]:1:query_cword}" 2>/dev/null
    )
    case "${result[0]}" in
        identities)
            COMPREPLY=()
            for line in "${result[@]:1}"; do
                printf -v quoted '%q' "$line"
                if [[ -n $open_quote ]]; then quoted=$line; fi
                if [[ ${COMP_WORDS[COMP_CWORD]} == --identity=* ]]; then
                    quoted="--identity=$quoted"
                fi
                COMPREPLY+=("$quoted")
            done
            ;;
        command)
            offset=$((${result[1]} + 1))
            if declare -F _comp_command_offset >/dev/null; then
                _comp_command_offset "$offset"
                return
            fi
            # Without bash-completion, already registered function completions
            # still work. Locals preserve the caller's completion context.
            local -a COMP_WORDS=("${COMP_WORDS[@]:offset}")
            local COMP_CWORD=$((COMP_CWORD - offset))
            command_name=${COMP_WORDS[0]}
            COMPREPLY=()
            if ((COMP_CWORD == 0)); then
                while IFS= read -r line; do COMPREPLY+=("$line"); done < <(compgen -c -- "$command_name")
                return
            fi
            spec=$(complete -p -- "$command_name" 2>/dev/null)
            if [[ $spec =~ ' -F '([^ ]+) ]]; then
                function_name=${BASH_REMATCH[1]}
                "$function_name" "$command_name" "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"
            else
                while IFS= read -r line; do COMPREPLY+=("$line"); done < <(compgen -f -- "${COMP_WORDS[COMP_CWORD]}")
            fi
            ;;
        *) _tmt_static "$@" ;;
    esac
}
if [[ ${BASH_VERSINFO[0]} -gt 4 || ${BASH_VERSINFO[0]} -eq 4 && ${BASH_VERSINFO[1]} -ge 4 ]]; then
    complete -F _tmt -o nosort tmt
else
    complete -F _tmt tmt
fi
