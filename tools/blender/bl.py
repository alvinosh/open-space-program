"""Send Python code to the Blender MCP addon socket (localhost:9876) and print the result.

Usage: python3 bl.py script.py [--preview DIR]   (runs the file's contents inside Blender)
       python3 bl.py -c "print(1)"

When running a file, `__file__` is defined for the script (so it can find its helper
modules) and `--preview DIR` defines OSP_PREVIEW_DIR, which the model scripts use to
render a preview PNG after exporting.
"""
import json, os, socket, sys


def run(code, timeout=600):
    s = socket.create_connection(("localhost", 9876), timeout=timeout)
    s.sendall(json.dumps({"type": "execute_code", "params": {"code": code}}).encode())
    buf = b""
    while True:
        chunk = s.recv(1 << 20)
        if not chunk:
            break
        buf += chunk
        try:
            return json.loads(buf)
        except ValueError:
            continue
    return json.loads(buf)


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[0] == "-c":
        code = args[1]
    else:
        path = os.path.abspath(args[0])
        header = f"__file__ = {path!r}\n"
        if "--preview" in args:
            header += f"OSP_PREVIEW_DIR = {os.path.abspath(args[args.index('--preview') + 1])!r}\n"
        code = header + open(path).read()
    reply = run(code)
    if reply.get("status") != "success":
        print("ERROR:", reply.get("message", reply))
        sys.exit(1)
    result = reply.get("result")
    print(result.get("result", result) if isinstance(result, dict) else result)
