"""Navigate actual connection controls without positional Tab assumptions."""
from workbench_smoke import wait_for

FIELDS = {'key': 'API key', 'endpoint': 'Endpoint', 'model': 'Default model',
           'protocol': 'Protocol', 'auth': 'Authentication', 'storage': 'Storage',
           'save': 'Save & choose model', 'advanced': 'Advanced settings',
           'forget': 'Forget app connection'}


def focus_field(session, name, ):
    label = FIELDS[name]
    for _ in range(16):
        if '> ' + label + ':' in session.display():
            return
        session.send('\t')
    raise AssertionError('Connection field not reachable: ' + label + '\n' + session.display())


def open_setup(session, route, advanced=False):
    # Route-qualified explicit entry is for independent transport fixtures;
    # unified-family selection itself is exercised by provider_ux_smoke.
    # Startup can still be loading/saving after the first terminal redraw.
    # Connection setup requires committed idle state, not merely echoed keys.
    wait_for(session, 'Storage: saved')
    session.command('connect ' + ('provider-openai' if route == 'openai' else route))
    wait_for(session, FIELDS['key'] + ':')
    if advanced and route != 'compatible':
        focus_field(session, 'advanced', )
        session.send('\r')
    focus_field(session, 'key', )


def fill_text(session, name, value, ):
    focus_field(session, name, )
    session.send('\x15')
    session.paste(value)
