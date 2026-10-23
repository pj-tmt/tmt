"""The ANSI cursor/erase decoder shared by Ops native terminal peers."""
import codecs
import re


class Screen:
    """Decode the cursor/erase subset written by the real Crossterm backend."""

    def __init__(self):
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.x = self.y = 0
        self.cursor_visible = False
        self.cells = [[" "] * 160 for _ in range(36)]

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b"):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if not match:
                    return
                self.pending = self.pending[match.end():]
                values, _, final = match.groups()
                if values.startswith("?"):
                    if values == "?1049" and final == "h":
                        self.cells = [[" "] * 160 for _ in range(36)]
                    if values == "?25" and final in ("h", "l"):
                        self.cursor_visible = final == "h"
                    continue
                if values.startswith((">", "<", "=")):
                    continue
                args = [int(v or 0) for v in values.split(";")] if values else [0]
                count = args[0] or 1
                if final in ("H", "f"):
                    self.y = max(0, args[0] - 1)
                    self.x = max(0, (args[1] if len(args) > 1 else 1) - 1)
                elif final == "G":
                    self.x = count - 1
                elif final == "A":
                    self.y = max(0, self.y - count)
                elif final == "B":
                    self.y += count
                elif final == "C":
                    self.x += count
                elif final == "D":
                    self.x = max(0, self.x - count)
                elif final == "J" and args[0] in (2, 3):
                    self.cells = [[" "] * 160 for _ in range(36)]
                elif final == "K" and self.y < 36:
                    start = 0 if args[0] in (1, 2) else self.x
                    end = self.x + 1 if args[0] == 1 else 160
                    self.cells[self.y][start:end] = [" "] * (end - start)
                continue
            char, self.pending = self.pending[0], self.pending[1:]
            if char == "\r":
                self.x = 0
            elif char == "\n":
                self.y += 1
            elif char >= " ":
                if self.y < 36 and self.x < 160:
                    self.cells[self.y][self.x] = char
                self.x += 1

    def contains(self, text):
        return any(text in "".join(row) for row in self.cells)

