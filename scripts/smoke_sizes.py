"""Terminal sizes for contract smoke drivers.

EV_SMOKE_SIZES="60x18" (comma-separated WxH) narrows a run; unset keeps each
driver's full default. CI runs every size on one full-profile target and the
compact size elsewhere, where layout pressure is highest.
"""
import os


def selected(defaults):
    """Filter `defaults` (sequence of items whose first element or self is (w, h))."""
    wanted = os.environ.get('EV_SMOKE_SIZES', '').strip()
    if not wanted:
        return list(defaults)
    sizes = {tuple(int(part) for part in item.lower().split('x')) for item in wanted.split(',') if item}

    def size(item):
        return item[0] if isinstance(item[0], tuple) else item

    chosen = [item for item in defaults if size(item) in sizes]
    assert chosen, f'EV_SMOKE_SIZES={wanted!r} selects none of {list(defaults)}'
    return chosen
