from pathlib import Path


class Deployer:
    def load(self):
        return sorted(Path(".").glob("contracts/*.sol"))

    def deploy(self, path):
        print(path)

    def run(self):
        for path in self.load():
            self.deploy(path)
