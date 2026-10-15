"""Job service: queue jobs and run them."""

from app.util import slugify


class JobQueue:
    """In-memory queue of pending jobs."""

    def __init__(self):
        self.items = []

    def push(self, job):
        self.items.append(slugify(job))
        return len(self.items)


def run_job(name):
    queue = JobQueue()
    return queue.push(name)
