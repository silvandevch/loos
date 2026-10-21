# LoOS Calculator

Native Win32 desktop calculator for LoOS. Supports decimal arithmetic, unary
signs, parentheses, addition, subtraction, multiplication and division. The
expression parser rejects malformed expressions, division by zero and
non-finite results. It uses only local computation and has no network or update
code.

Build it with `./loos source-build windows-mingw`; the result is
`OUT_ROOT/cmake/windows-mingw/apps/LoCalc.exe`.
