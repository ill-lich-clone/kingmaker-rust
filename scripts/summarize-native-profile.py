"""Summarize a streamed native profile, including an abruptly terminated run."""
import argparse
import json
from pathlib import Path


def summarize(profile_path: Path, process_path: Path) -> dict:
    process = json.loads(process_path.read_text(encoding="utf-8-sig"))
    active, groups, counters = {}, {}, {}
    header = footer = None
    corrupted = truncated = False
    errors = []
    last_id = 0
    total_self = 0
    completed = 0
    with profile_path.open("rb") as stream:
        pending = stream.readline()
        while pending:
            following = stream.readline()
            try:
                row = json.loads(pending)
            except (ValueError, UnicodeDecodeError):
                if not following and not pending.endswith(b"\n"):
                    truncated = True
                else:
                    corrupted = True
                    errors.append("invalid JSON within stream")
                break
            try:
                if footer is not None:
                    raise ValueError("data after footer")
                event = row["event"]
                if event == "header":
                    if header is not None or row["schema"] != 1:
                        raise ValueError("invalid header")
                    header = row
                elif header is None:
                    raise ValueError("event before header")
                elif event == "begin":
                    ident = row["id"]
                    parent = row["parent_id"]
                    if ident <= last_id or len(active) >= 64 or parent != next(reversed(active), None):
                        raise ValueError("invalid scope nesting or ID")
                    last_id = ident
                    row["parent_name"] = active[parent]["name"] if parent is not None else None
                    active[ident] = row
                elif event == "end":
                    ident = row["id"]
                    if ident != next(reversed(active), None):
                        raise ValueError("scope closed out of order")
                    begin = active.pop(ident)
                    elapsed, own = row["elapsed_ns"], row["self_ns"]
                    if not (0 <= own <= elapsed):
                        raise ValueError("invalid scope time")
                    key = (begin["parent_name"], begin["name"])
                    if key not in groups:
                        if len(groups) >= 512:
                            raise ValueError("too many scope categories")
                        groups[key] = dict(parent=key[0], name=key[1], calls=0, inclusive_ns=0, self_ns=0, outcomes={})
                    group = groups[key]
                    group["calls"] += 1
                    group["inclusive_ns"] += elapsed
                    group["self_ns"] += own
                    outcome = row["outcome"]
                    group["outcomes"][outcome] = group["outcomes"].get(outcome, 0) + 1
                    completed += 1
                    total_self += own
                    for name, value in row.get("counters", {}).items():
                        if len(counters) >= 13 and name not in counters:
                            raise ValueError("too many counters")
                        if not isinstance(value, int) or value < 0:
                            raise ValueError("invalid counter")
                        counters[name] = counters.get(name, 0) + value
                elif event == "footer":
                    footer = row
                else:
                    raise ValueError("unknown event")
            except (KeyError, TypeError, ValueError) as error:
                corrupted = True
                errors.append(str(error))
                break
            pending = following
    profile_pid = header.get("pid") if header else None
    process_pid = process.get("pid")
    association = "unverified"
    if isinstance(profile_pid, int) and profile_pid > 0 and isinstance(process_pid, int) and process_pid > 0:
        association = "matched" if profile_pid == process_pid else "mismatch"
    if association != "matched":
        errors.append("profile/process identity " + association)
    success = process.get("outcome") == "completed" and process.get("exit_code") == 0
    complete = bool(header and footer and footer.get("complete") and not active and not corrupted and not truncated and success and association == "matched")
    return dict(schema=1, complete=complete, process=process if association == "matched" else None,
                process_association=association, profile_pid=profile_pid, supplied_process_pid=process_pid, corrupted=corrupted,
                truncated_tail=truncated, errors=errors, footer=footer,
                last_open_scopes=list(active.values()), completed_scopes=completed,
                total_self_ns=total_self, counters=counters,
                scopes=sorted(groups.values(), key=lambda g: g["self_ns"], reverse=True),
                allocation_bytes=None, physical_disk_bytes=None,
                coverage="Only completed scopes contribute times/counters; unfinished scopes are lower-bound evidence. SQLite payload bytes are logical, not physical disk IO. Self times avoid child double counting; inclusive times must not be summed.")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--process", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = summarize(args.profile, args.process)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
    print(json.dumps({k: result[k] for k in ("complete", "completed_scopes", "corrupted", "truncated_tail") }))

if __name__ == "__main__":
    main()
