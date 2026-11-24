// The <stdexcept> members that libc++abi provides on macOS (what() and the destructors, which
// also emit the classes' vtables and type_info). Prism's C++ ABI runtime is C, so libc++ itself
// carries them; same definitions as libc++'s stdexcept_default.ipp.
#include <stdexcept>
#include "include/refstring.h"

namespace std {
const char* logic_error::what() const noexcept { return __imp_.c_str(); }
const char* runtime_error::what() const noexcept { return __imp_.c_str(); }
logic_error::~logic_error() noexcept {}
domain_error::~domain_error() noexcept {}
invalid_argument::~invalid_argument() noexcept {}
length_error::~length_error() noexcept {}
out_of_range::~out_of_range() noexcept {}
runtime_error::~runtime_error() noexcept {}
range_error::~range_error() noexcept {}
overflow_error::~overflow_error() noexcept {}
underflow_error::~underflow_error() noexcept {}
} // namespace std
