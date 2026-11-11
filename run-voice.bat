@echo off
REM Voice chain demo launcher: use the Python env that has sherpa-onnx/kokoro deps.
REM Falls back to plain `python` if the managed env is not present.
set MANAGED=D:\KimiData\daimon-share\daimon\runtime\python\.venv\Scripts\python.exe
if exist "%MANAGED%" (
  "%MANAGED%" scripts\voice_chain_demo.py %*
) else (
  python scripts\voice_chain_demo.py %*
)
