
_tmt() {
    local -a result
    local offset
    result=("${(@f)$("${(Q)words[1]}" __complete -- "${(@Q)words[2,CURRENT]}" 2>/dev/null)}")
    case $result[1] in
        identities)
            compset -P '--identity='
            compadd -V identities -- "${(@)result[2,-1]}"
            ;;
        command)
            offset=$((result[2] + 1))
            shift offset words
            (( CURRENT -= offset ))
            _normal -p tmt
            ;;
        *) _tmt_static "$@" ;;
    esac
}
if [[ $funcstack[1] == _tmt ]]; then
    _tmt "$@"
else
    compdef _tmt tmt
fi
