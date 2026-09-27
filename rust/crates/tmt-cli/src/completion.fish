
function __tmt_candidates
    set -l prior (commandline -opc)
    set -l current (commandline -ct)
    set -l decoded (string unescape -- "$current")
    set -l executable $prior[1]
    set -l result ($executable __complete -- $prior[2..-1] "$decoded" 2>/dev/null)
    switch "$result[1]"
        case identities
            if string match -q -- '--identity=*' "$current"
                for candidate in $result[2..-1]
                    printf '%s\n' "--identity=$candidate"
                end
            else
                printf '%s\n' $result[2..-1]
            end
        case command
            set -l first (math $result[2] + 2)
            set -l line (string join ' ' -- (string escape -- $prior[$first..-1]))
            if test -n "$line"
                set line "$line "
            end
            complete -C "$line$current"
        case '*'
            set -l line (string join ' ' -- __tmt_static (string escape -- $prior[2..-1]))
            complete -C "$line $current"
    end
end
complete -c tmt -f -k -a '(__tmt_candidates)'
