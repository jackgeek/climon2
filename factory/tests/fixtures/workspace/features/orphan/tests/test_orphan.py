import subprocess


def test_leaves_a_detached_process_behind() -> None:
    subprocess.Popen(["sleep", "2999"], start_new_session=True)
