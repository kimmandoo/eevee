#!/usr/bin/env python3
"""Actual PTY sprite transport/input acceptance; not a GUI-terminal certification.

Requires the terminal_smoke dependencies and Pillow. The supplied binary and
verified sprite cache must exist. Replies emulate terminal capabilities, but the
real application negotiates, encodes, paints, clears and preserves input.
"""
import argparse
import base64
import json
from pathlib import Path
import re
import sqlite3
import tempfile
import time

from PIL import Image
import pyte
from terminal_smoke import Session, terminal_environment, ESC


def sixels(text):
    return re.findall(r'\x1bP[0-9;]*q(.*?)\x1b\\', text, re.S)


def decode_sixel(data):
    raster = re.search(r'"\d+;\d+;(\d+);(\d+)', data)
    assert raster, 'Sixel payload has no raster dimensions'
    width, height = map(int, raster.groups())
    assert 0 < width <= 2048 and 0 < height <= 2048
    image = Image.new('RGB', (width, height))
    palette, current, x, y, at = {}, 0, 0, 0, 0
    while at < len(data):
        char = data[at]
        if char in '#"':
            match = re.match(r'[0-9;]+', data[at + 1:])
            assert match
            values = list(map(int, match.group().split(';')))
            if char == '#':
                current = values[0]
                if len(values) > 1:
                    assert len(values) == 5 and values[1] == 2, 'expected RGB Sixel palette'
                    palette[current] = tuple(round(component * 255 / 100) for component in values[2:])
            at += len(match.group()) + 1
            continue
        count = 1
        if char == '!':
            match = re.match(r'\d+', data[at + 1:])
            assert match
            count = int(match.group())
            assert 0 < count <= 2048
            at += len(match.group()) + 1
            char = data[at]
        if '?' <= char <= '~':
            bits = ord(char) - 63
            for column in range(x, x + count):
                for bit in range(6):
                    if bits & (1 << bit) and column < width and y + bit < height:
                        image.putpixel((column, y + bit), palette[current])
            x += count
        elif char == '$':
            x = 0
        elif char == '-':
            x, y = 0, y + 6
        at += 1
    return image


def kitty_images(text):
    active, images = None, []
    for header, payload in re.findall(r'\x1b_G([^;]*);([^\x1b]*)\x1b\\', text):
        fields = dict(piece.split('=', 1) for piece in header.split(',') if '=' in piece)
        if fields.get('a') == 't':
            assert fields.get('f') == '32', 'expected original RGBA Kitty transmission'
            active = (int(fields['s']), int(fields['v']), [])
        if active is not None and payload:
            active[2].append(payload)
            if fields.get('m', '0') == '0':
                width, height, chunks = active
                pixels = base64.b64decode(''.join(chunks))
                assert len(pixels) == width * height * 4
                images.append(Image.frombytes('RGBA', (width, height), pixels).convert('RGB'))
                active = None
    return images


def original_pixels(rendered, source):
    """Every opaque source pixel survives at an integer scale, within Sixel rounding."""
    source = source.crop(source.getbbox())
    for scale in range(1, min(rendered.width // source.width, rendered.height // source.height) + 1):
        expected = source.resize((source.width * scale, source.height * scale), Image.Resampling.NEAREST)
        left = (rendered.width - expected.width) // 2
        top = (rendered.height - expected.height) // 2
        maximum = 0
        for y in range(expected.height):
            for x in range(expected.width):
                pixel = expected.getpixel((x, y))
                if pixel[3] != 255:
                    continue
                actual = rendered.getpixel((left + x, top + y))
                maximum = max(maximum, *(abs(a - b) for a, b in zip(actual[:3], pixel[:3])))
        if maximum <= 3:
            return {'source_subject_pixels': list(source.size), 'rendered_pixels': list(rendered.size),
                    'integer_scale': scale, 'maximum_rgb_error': maximum}
    raise AssertionError('native payload lost/resampled original sprite pixels: ' + str(rendered.size))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/ev')
    parser.add_argument('--assets-dir', required=True)
    parser.add_argument('--output', default='target/graphics-smoke')
    args = parser.parse_args()
    binary, assets, output = Path(args.binary).resolve(), Path(args.assets_dir).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = Image.open(assets / '133.png').convert('RGBA')
    results = {'status': 'failed', 'cases': [], 'limits': ['PTY capability responder, not real terminal GUI rendering']}
    try:
        with tempfile.TemporaryDirectory(prefix='ev-graphics-') as temporary:
            root = Path(temporary)
            for protocol in ('sixel', 'kitty', 'unsupported', 'cells', 'off'):
                data = root / protocol
                mode = protocol if protocol in ('cells', 'off') else 'auto'
                environment = terminal_environment('truecolor')
                session = Session([str(binary), '--data-dir', str(data), '--assets-dir', str(assets),
                                   '--images', mode], root, environment, (120, 40), output / protocol)

                def text_screen():
                    # Pyte does not implement APC/DCS images. Ignore their payloads
                    # only for text assertions; independently decode/verify pixels.
                    screen = pyte.Screen(120, 40)
                    text = re.sub(r'\x1b(?:P|_)[^\x1b]*\x1b\\', '', session.text)
                    pyte.Stream(screen).feed(text)
                    return '\n'.join(screen.display)

                def until(predicate, label):
                    deadline = time.monotonic() + 8
                    while not predicate():
                        assert session.alive() and time.monotonic() < deadline, label
                        session.pump(.03)

                def respond():
                    if protocol == 'sixel':
                        session.send(ESC + '[?62;4c')
                    else:
                        session.send(ESC + '_Gi=314159;OK' + ESC + '\\')
                    # Split metrics across reads; arriving after the bounded query
                    # deadline still belongs to discovery, never the editor.
                    session.send(ESC + '[6;20;')
                    session.send('10t')

                try:
                    until(lambda: 'Storage: saved' in session.display(), 'initial local UI')
                    if mode == 'auto':
                        until(lambda: ESC + '[16t' in session.text, 'pixel metrics query')
                    else:
                        assert ESC + '[16t' not in session.text, 'explicit mode unexpectedly queried graphics'
                    session.send('graphics draft')
                    if protocol in ('sixel', 'kitty'):
                        respond()
                        images = lambda: [decode_sixel(value) for value in sixels(session.text)] if protocol == 'sixel' else kitty_images(session.text)
                        until(lambda: bool(images()), 'native sprite payload')
                        evidence = original_pixels(images()[0], source)
                        images()[0].save(output / (protocol + '-decoded.png'))
                        assert 'U=1' not in session.text, 'Kitty virtual placeholders require an unprobed capability'
                        before = len(session.text)
                        session.send(ESC + 'OP')
                        until(lambda: 'GETTING STARTED' in text_screen(), 'help overlay')
                        changed = session.text[before:]
                        if protocol == 'sixel':
                            assert ESC + '[2J' in changed, 'Sixel image not erased before overlay'
                            assert not sixels(changed), 'Sixel painted through help overlay'
                        else:
                            assert 'a=d' in changed, 'Kitty placement not removed for overlay'
                        session.send(ESC)
                        session.send('g')  # Resume the retained composer draft after help.
                        session.resize((100, 30))
                        respond()
                        session.pump(.4)
                    else:
                        session.pump(.7)
                        assert not sixels(session.text) and not kitty_images(session.text), 'unsupported mode emitted native image'
                        if protocol != 'cells':
                            assert '▀' not in session.text and '▄' not in session.text, 'automatic fallback rendered a coarse mosaic'
                        evidence = {'mode': mode, 'native_image_emitted': False}
                    session.send('\r')
                    database = data / 'workbench.sqlite3'
                    def goal_saved():
                        with sqlite3.connect(database) as db:
                            return db.execute('SELECT goal FROM tasks').fetchall() == [('graphics draft',)]
                    until(goal_saved, 'typed text was lost, duplicated or polluted by terminal replies')
                    session.send('\x03'); session.send(ESC + '[C'); session.send('\r')
                    assert session.wait_exit() == 0
                    results['cases'].append({'protocol': protocol, **evidence, 'input_preserved': True, 'terminal_restored': True})
                    print('passed:', protocol, evidence, flush=True)
                finally:
                    session.close()
        results['status'] = 'passed'
    finally:
        (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
