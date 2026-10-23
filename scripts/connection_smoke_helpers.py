"""Navigate actual connection controls without positional Tab assumptions."""
from workbench_smoke import wait_for

FIELDS = {
    'en': {'key': 'API key', 'endpoint': 'Endpoint', 'model': 'Default model',
           'protocol': 'Protocol', 'auth': 'Authentication', 'storage': 'Storage',
           'save': 'Save & choose model', 'advanced': 'Advanced settings',
           'forget': 'Forget app connection'},
    'ko': {'key': 'API 키', 'endpoint': '엔드포인트', 'model': '기본 모델',
           'protocol': '프로토콜', 'auth': '인증', 'storage': '저장 방식',
           'save': '저장 후 모델 선택', 'advanced': '고급 설정', 'forget': '앱 연결 삭제'},
}


def focus_field(session, name, language='en'):
    label = FIELDS[language][name]
    for _ in range(16):
        if '> ' + label + ':' in session.display():
            return
        session.send('\t')
    raise AssertionError('Connection field not reachable: ' + label + '\n' + session.display())


def open_setup(session, route, language='en', advanced=False):
    # Route-qualified explicit entry is for independent transport fixtures;
    # unified-family selection itself is exercised by provider_ux_smoke.
    session.command('connect ' + ('provider-openai' if route == 'openai' else route))
    wait_for(session, FIELDS[language]['key'] + ':')
    if advanced and route != 'compatible':
        focus_field(session, 'advanced', language)
        session.send('\r')
    focus_field(session, 'key', language)


def fill_text(session, name, value, language='en'):
    focus_field(session, name, language)
    session.send('\x15')
    session.paste(value)
