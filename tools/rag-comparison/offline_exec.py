"""Run a local comparison command with networking denied on Linux.

This launcher installs an inherited seccomp filter before loading a parser.
It opens no listener and fails closed if the operating-system filter cannot
be installed. It is a network restriction, not a filesystem sandbox.

Functions:
    deny_network: Install an inherited network syscall restriction.
    verify_denial: Check socket creation in this process and a child.
    main: Parse arguments and execute the restricted command.
"""

import argparse
import ctypes
import errno
import logging
import os
from pathlib import Path
import resource
import socket
import subprocess
import sys


SCMP_ACT_ALLOW = 0x7FFF0000
SCMP_ACT_ERRNO = 0x00050000
DENIED_SYSCALLS = (
    "socket", "socketpair", "connect", "sendto", "sendmsg", "sendmmsg",
    "io_uring_setup", "io_uring_enter", "io_uring_register",
)


def deny_network() -> None:
    """Install an inherited network syscall restriction.

    Block new sockets and network transmission, including io_uring paths.
    The restriction survives exec and applies to future child processes.

    Args:
        none (None): This function does not accept arguments.

    Returns:
        None: This function does not return a value.
    """
    if sys.platform != "linux":
        raise RuntimeError("Offline execution requires Linux libseccomp.")
    library = ctypes.CDLL("libseccomp.so.2", use_errno=True)
    library.seccomp_init.argtypes = [ctypes.c_uint32]
    library.seccomp_init.restype = ctypes.c_void_p
    library.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
    library.seccomp_syscall_resolve_name.restype = ctypes.c_int
    library.seccomp_rule_add.argtypes = [
        ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int, ctypes.c_uint,
    ]
    library.seccomp_rule_add.restype = ctypes.c_int
    library.seccomp_load.argtypes = [ctypes.c_void_p]
    library.seccomp_load.restype = ctypes.c_int
    library.seccomp_release.argtypes = [ctypes.c_void_p]
    context = library.seccomp_init(SCMP_ACT_ALLOW)
    if not context:
        raise RuntimeError("Could not initialize the network filter.")
    try:
        for name in DENIED_SYSCALLS:
            number = library.seccomp_syscall_resolve_name(name.encode())
            if number < 0:
                raise RuntimeError(f"Cannot restrict syscall {name}.")
            result = library.seccomp_rule_add(
                context, SCMP_ACT_ERRNO | errno.EPERM, number, 0
            )
            if result != 0:
                raise RuntimeError(f"Cannot restrict syscall {name}.")
        if library.seccomp_load(context) != 0:
            raise RuntimeError("Cannot load the network filter.")
    finally:
        library.seccomp_release(context)


def verify_denial() -> None:
    """Check socket creation in this process and a child.

    Attempt IPv4, IPv6 and Unix sockets without making external requests.
    Repeat the checks in an executed child to verify filter inheritance.

    Args:
        none (None): This function does not accept arguments.

    Returns:
        None: This function does not return a value.
    """
    for family in (socket.AF_INET, socket.AF_INET6, socket.AF_UNIX):
        try:
            connection = socket.socket(family, socket.SOCK_STREAM)
        except OSError as error:
            if error.errno != errno.EPERM:
                raise RuntimeError(
                    "Unexpected network-filter error."
                ) from error
        else:
            connection.close()
            raise RuntimeError("Network filter did not block socket creation.")
    child = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), "--check-inherited"],
        check=False,
        close_fds=True,
    )
    if child.returncode != 0:
        raise RuntimeError("Child process did not inherit the network filter.")


def main() -> None:
    """Parse arguments and execute the restricted command.

    Set offline model flags and close inherited descriptors before exec.
    Never load document-processing dependencies before applying the filter.

    Args:
        none (None): This function does not accept arguments.

    Returns:
        None: This function replaces the process or ends after a self-test.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Graphite offline comparison launcher. Run a local command with "
            "networking denied by Linux seccomp. Inputs are a command and "
            "its arguments; outputs are that command's local outputs.\n"
            "Flow:\n1. Install the network filter.\n"
            "2. Set offline flags and close inherited descriptors.\n"
            "3. Execute the command without a shell.\n"
            "Requires libseccomp; fails closed if unavailable.\n"
            "Example: python offline_exec.py --self-test"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--check-inherited", action="store_true",
                        help=argparse.SUPPRESS)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    if args.check_inherited:
        try:
            connection = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        except OSError as error:
            if error.errno != errno.EPERM:
                raise
            return
        connection.close()
        raise RuntimeError("Inherited filter allowed a network socket.")
    deny_network()
    if args.self_test:
        verify_denial()
        logging.info("PASS: sockets denied in parent and executed child.")
        return
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("Provide a command or --self-test.")
    for name in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE",
                 "HF_HUB_DISABLE_TELEMETRY"):
        os.environ[name] = "1"
    os.environ["DO_NOT_TRACK"] = "1"
    maximum = resource.getrlimit(resource.RLIMIT_NOFILE)[0]
    os.closerange(3, int(maximum))
    os.execvp(command[0], command)


if __name__ == "__main__":
    main()
