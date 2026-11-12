"""Guard the UI-only target contract across source, frontend, tests and docs."""
import ipaddress
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]

class TargetContractTest(unittest.TestCase):
    def test_no_ip_address_literals_in_source_frontend_tests_or_docs(self):
        paths = [*ROOT.joinpath('src').rglob('*.rs'), *ROOT.joinpath('web').glob('*'),
                 *ROOT.joinpath('tests').glob('*.py'), *ROOT.joinpath('tests').glob('*.cjs'),
                 ROOT/'README.md', ROOT/'PROGRESS.md']
        for path in paths:
            text=path.read_text()
            candidates=re.findall(r'(?:\d{1,3}\.){3}\d{1,3}', text)
            candidates += re.findall(r'[\"\']([a-fA-F\d:]+)[\"\']',text)
            for value in candidates:
                try: ipaddress.ip_address(value)
                except ValueError: continue
                self.fail(f'{path.relative_to(ROOT)} contains a direct IP literal; use UI input or platform loopback constants')

    def test_no_alternative_target_configuration_path(self):
        self.assertFalse(ROOT.joinpath('src/config.rs').exists())
        self.assertFalse(ROOT.joinpath('src/monitor.rs').exists())
        self.assertFalse(list(ROOT.joinpath('config').glob('*.example.toml')))
        self.assertNotIn('innerHTML',ROOT.joinpath('web/app.js').read_text())

if __name__ == '__main__': unittest.main()
