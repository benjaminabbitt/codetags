# Name sites for the Python provider (codetags-ingest provider::python).
#
# Usage: python python_sites.py <root> <output>
# Reads relative paths under <root>, one per line, UTF-8, from stdin. Writes
# one JSON object per line to <output> for each call or attribute reference
# in those files: every attribute read (`x.name`), and every bare name that is
# called or used as a decorator. Fields: file, line (0-based), start and end
# (UTF-16 code units, as Pyright and so scip-python count), name, called.
# A file that does not parse is an error: exit 3, `<file>:<line>: <message>`
# on stderr. Standard library only; Python 3.8 or later.

import ast
import importlib.util
import json
import sys


def utf16(text):
    return len(text.encode('utf-16-le')) // 2


def is_name_char(char):
    return char.isalnum() or char == '_'


def sites(rel, data):
    text = importlib.util.decode_source(data)
    try:
        tree = ast.parse(data, filename=rel)
    except SyntaxError as error:
        sys.stderr.write('%s:%s: %s\n' % (rel, error.lineno, error.msg))
        sys.exit(3)
    lines = text.split('\n')
    called = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            called.add(id(node.func))
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            called.update(id(decorator) for decorator in node.decorator_list)
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and isinstance(node.ctx, ast.Load):
            # The attribute's name ends the node; walk back over it.
            line = node.end_lineno - 1
            prefix = lines[line].encode('utf-8')[:node.end_col_offset].decode('utf-8')
            begin = len(prefix)
            while begin > 0 and is_name_char(prefix[begin - 1]):
                begin -= 1
            name, start, end = prefix[begin:], utf16(prefix[:begin]), utf16(prefix)
        elif isinstance(node, ast.Name) and id(node) in called:
            line = node.lineno - 1
            raw = lines[line].encode('utf-8')
            name = node.id
            start = utf16(raw[:node.col_offset].decode('utf-8'))
            end = utf16(raw[:node.end_col_offset].decode('utf-8'))
        else:
            continue
        found.append((line, start, end, name, id(node) in called))
    return sorted(found)


def main():
    root, output = sys.argv[1], sys.argv[2]
    paths = [path for path in sys.stdin.buffer.read().decode('utf-8').splitlines() if path]
    with open(output, 'w', encoding='utf-8', newline='\n') as out:
        for rel in paths:
            with open(root + '/' + rel, 'rb') as source:
                data = source.read()
            for line, start, end, name, is_called in sites(rel, data):
                out.write(json.dumps({
                    'file': rel, 'line': line, 'start': start, 'end': end,
                    'name': name, 'called': is_called,
                }) + '\n')


main()
