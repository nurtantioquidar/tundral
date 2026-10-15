import os


def run():
    return 1


def shared():
    return 2


class Alpha:
    def go(self):
        return 3

    def solo(self):
        return 4


def edges():
    def inner_helper():
        return 5

    return inner_helper


class Data:
    pass


class HTTP:
    pass


def str():
    return "denylisted builtin name"
