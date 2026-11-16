#!/usr/bin/env python3
"""把发布说明里手工折出来的软换行合并成「一段一行、一项一行」。

为什么需要：GitHub 的 Release 页面（和 issue / PR 一样）把单个换行当成真的
换行，`git tag -a -F notes.txt` 里按 80 列折过的段落，在页面上每行只走半截就
断，右边留一大片空白。反过来说，README 文件页面又会把软换行合成空格，中文
段落里因此多出不该有的缝。

只动「软换行」：段落内、列表项内的续行合并回上一行。代码块、表格、标题、
水平线、缩进代码块原样保留。中文之间直接相连，中英边界补一个空格；中文标点
两边不补。

用法：
    python3 tools/notes-reflow.py notes.txt          # 就地重排
    python3 tools/notes-reflow.py --check notes.txt  # 只检查，有折行就退出码 1
    python3 tools/notes-reflow.py --diff notes.txt   # 重排并把改动打到 stdout
"""

import difflib
import re
import sys

# 汉字（含日文假名、谚文）：两边都是它就直接相连。
CJK_CHAR = re.compile(r"[\u3400-\u9fff\uf900-\ufaff\u3040-\u30ff\uac00-\ud7af]")
# 中日韩标点与全角符号：它自带视觉间隔，两边都不补空格。
CJK_PUNCT = re.compile(
    r"[\u3000-\u303f\uff01-\uff0f\uff1a-\uff20\uff3b-\uff40\uff5b-\uff65"
    r"\u2014\u2026\u00b7\u2018\u2019\u201c\u201d]"
)
LIST_ITEM = re.compile(r"^(\s*)([-*+]|\d+[.)])\s+")
HEADING = re.compile(r"^#{1,6}\s")
HR = re.compile(r"^(-{3,}|\*{3,}|_{3,})$")


def _is_cjk(ch):
    return bool(CJK_CHAR.match(ch))


def _is_cjk_punct(ch):
    return bool(CJK_PUNCT.match(ch))


def joiner(left, right):
    """两行接起来时中间该不该补空格。"""
    if not left or not right:
        return ""
    a, b = left[-1], right[0]
    # 中文标点自己带间隔：`绕过网段、` + `TUN 网卡名` -> `绕过网段、TUN 网卡名`
    if _is_cjk_punct(a) or _is_cjk_punct(b):
        return ""
    if _is_cjk(a) and _is_cjk(b):
        return ""
    return " "


def reflow(text):
    """返回重排后的文本；只合并软换行，其它结构原样保留。"""
    out = []
    in_fence = False
    kind = None  # 'para' | 'item' | None
    buf = ""

    def flush():
        nonlocal buf, kind
        if buf:
            out.append(buf)
        buf, kind = "", None

    for raw in text.split("\n"):
        line = raw.rstrip()
        stripped = line.strip()

        if in_fence:
            out.append(line)
            if stripped.startswith("```"):
                in_fence = False
            continue

        if stripped.startswith("```"):
            flush()
            in_fence = True
            out.append(line)
            continue

        if stripped == "":
            flush()
            out.append("")
            continue

        if stripped.startswith("|") or HEADING.match(stripped) or HR.match(stripped):
            flush()
            out.append(line)
            continue

        # 缩进 4 格且不是列表续行：缩进代码块，原样保留。
        if kind is None and line.startswith("    "):
            flush()
            out.append(line)
            continue

        if stripped.startswith("<"):
            flush()
            out.append(line)
            continue

        if LIST_ITEM.match(line):
            flush()
            buf, kind = stripped, "item"
            continue

        if kind is not None:
            buf = buf + joiner(buf, stripped) + stripped
        else:
            buf, kind = stripped, "para"

    flush()
    # 末尾只留一个换行：split("\n") 已经把结尾换行变成最后一个空串了。
    out_text = "\n".join(out).rstrip("\n")
    return out_text + ("\n" if text.endswith("\n") else "")


def main(argv):
    check = "--check" in argv
    show_diff = "--diff" in argv
    paths = [a for a in argv if not a.startswith("--")]
    if len(paths) != 1 or (check and show_diff):
        sys.stderr.write(__doc__)
        return 2
    path = paths[0]

    with open(path, encoding="utf-8") as f:
        old = f.read()
    new = reflow(old)

    if new == old:
        print(f"{path}: 没有软换行，不用改")
        return 0

    if check:
        print(f"{path}: 有手工折行（{len(old.splitlines())} 行 -> {len(new.splitlines())} 行），需要重排")
        return 1

    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(new)
    print(f"{path}: 已重排，{len(old.splitlines())} 行 -> {len(new.splitlines())} 行")
    if show_diff:
        for line in difflib.unified_diff(
            old.splitlines(True), new.splitlines(True), "notes.txt（原）", "notes.txt（重排后）"
        ):
            sys.stdout.write(line)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
