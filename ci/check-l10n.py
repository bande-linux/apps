#!/usr/bin/env python3
import pathlib
import re
import sys

ENFORCED = set(pathlib.Path(__file__).with_name("l10n-complete").read_text().split())
MESSAGE = re.compile(r"^(-?[a-zA-Z][a-zA-Z0-9_-]*) *=")
ATTRIBUTE = re.compile(r"^\s+\.[a-zA-Z][a-zA-Z0-9_-]* *=")
VARIABLE = re.compile(r"\{\s*\$([a-zA-Z][a-zA-Z0-9_-]*)")


def parse(path, problems):
    messages = {}
    current = None
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = MESSAGE.match(line)
        if match:
            current = match.group(1)
            if current in messages:
                problems.append(f"{path}:{number}: duplicate message {current}")
            messages[current] = set(VARIABLE.findall(line))
        elif not line.strip() or line.startswith("#"):
            pass
        elif line[0] in " \t}" and current is not None:
            messages[current] |= set(VARIABLE.findall(line))
        else:
            problems.append(f"{path}:{number}: not Fluent: {line[:60]}")
        if line.count("{") < line.count("}") and line[0] not in " \t}":
            problems.append(f"{path}:{number}: unbalanced braces")
    return messages


def main():
    problems = []
    report = {}
    roots = sys.argv[1:] or ["."]
    for root in roots:
        for english in sorted(pathlib.Path(root).rglob("locales/en-US/*.ftl")):
            if "target" in english.parts or english.stem == "fixture":
                continue
            base = parse(english, problems)
            for translated in sorted(english.parent.parent.glob(f"*/{english.name}")):
                language = translated.parent.name
                if language == "en-US":
                    continue
                messages = parse(translated, problems)
                counts = report.setdefault(language, [0, 0])
                counts[1] += len(base)
                for name, variables in base.items():
                    if name not in messages:
                        if language in ENFORCED:
                            problems.append(f"{translated}: missing {name}")
                        continue
                    counts[0] += 1
                    extra = messages[name] - variables
                    if extra:
                        problems.append(f"{translated}: {name} uses unknown {sorted(extra)}")
                for name in messages.keys() - base.keys():
                    problems.append(f"{translated}: {name} is not in English")
            for language in ENFORCED:
                if not (english.parent.parent / language / english.name).exists():
                    problems.append(f"{english}: no {language} file")
    for language, (done, total) in sorted(report.items()):
        print(f"{language}: {done} of {total}")
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
