#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-3-Clause
"""Render a LAVA job definition from this directory and print it.

Usage:
  render.py [-D KEY=VALUE]... JOB

A job line of the form "# @INCLUDE FILE@" is replaced with the contents of
FILE (relative to the job), indented like that line. Every @KEY@ in the
result is then replaced with the matching -D value.
"""

import argparse
import os
import re
import sys


def include(definition, basedir):
    """Splice in the files named by "# @INCLUDE FILE@" lines."""
    def splice(match):
        indent = match.group(1)
        with open(os.path.join(basedir, match.group(2))) as f:
            return ''.join(indent + line if line.strip() else line
                           for line in f)
    return re.sub(r'^([ ]*)# @INCLUDE (\S+)@[ ]*\n', splice, definition,
                  flags=re.M)


def render(definition, defines):
    for key, value in defines.items():
        definition = definition.replace('@%s@' % key, value)
    left = sorted(set(re.findall(r'@[A-Z][A-Z0-9_]*@', definition)))
    if left:
        sys.exit('unresolved placeholders: ' + ' '.join(left))
    return definition


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('job', help='LAVA job definition (YAML)')
    parser.add_argument('-D', '--define', action='append', default=[],
                        metavar='KEY=VALUE', help='replace @KEY@ in the job')
    args = parser.parse_args()

    defines = dict(d.split('=', 1) for d in args.define)
    with open(args.job) as f:
        definition = include(f.read(), os.path.dirname(args.job))
    print(render(definition, defines), end='')


if __name__ == '__main__':
    main()
