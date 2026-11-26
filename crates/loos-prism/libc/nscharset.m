/* NSCharacterSet / NSMutableCharacterSet, the NSString methods that take one (trimming, splitting,
 * searching, percent encoding) and NSScanner. A set is a bitmap over the Basic Multilingual Plane
 * (65536 bits) plus an "everything above it" flag, which is what Cocoa code uses in practice. The
 * predefined sets use the Unicode ranges of the common scripts (Latin, Greek, Cyrillic, Hebrew,
 * Arabic, CJK, kana, Hangul). */
#include "foundation.h"

void *calloc(size_t, size_t);
void *malloc(size_t);
void free(void *);
void *memcpy(void *, const void *, size_t);
int memcmp(const void *, const void *, size_t);
long long strtoll(const char *, char **, int);
unsigned long long strtoull(const char *, char **, int);
double strtod(const char *, char **);

#define BMP_BYTES 8192

@interface NSCharacterSet : NSObject { @public uint8_t *_bits; BOOL _astral; }
@end
@interface NSMutableCharacterSet : NSCharacterSet
@end
@interface NSString (PrismCharsetUses)
+ (id)stringWithUTF8String:(const char *)s;
- (id)substringWithRange:(NSRange)r;
@end

static inline BOOL has(NSCharacterSet *s, uint32_t c) { return c > 0xffff ? s->_astral : (s->_bits[c >> 3] >> (c & 7)) & 1; }
static inline void put(NSCharacterSet *s, uint32_t c, BOOL on) {
    if (c > 0xffff) { s->_astral = on; return; }
    if (on) s->_bits[c >> 3] |= 1 << (c & 7); else s->_bits[c >> 3] &= ~(1 << (c & 7));
}

static NSCharacterSet *make(Class k) {
    NSCharacterSet *s = [k alloc];
    s->_bits = calloc(1, BMP_BYTES);
    return s;
}

/* A predefined set, built once from a predicate over the BMP. */
static NSCharacterSet *predefined(NSCharacterSet **slot, BOOL (*pred)(uint32_t), BOOL astral) {
    if (!*slot) {
        NSCharacterSet *s = make([NSCharacterSet class]);
        for (uint32_t c = 0; c < 0x10000; c++) if (pred(c)) put(s, c, YES);
        s->_astral = astral;
        *slot = s; /* never freed */
    }
    return *slot;
}

static BOOL p_ws(uint32_t c) { return c == ' ' || c == '\t' || c == 0xa0 || c == 0x1680 || (c >= 0x2000 && c <= 0x200a) || c == 0x202f || c == 0x205f || c == 0x3000; }
static BOOL p_nl(uint32_t c) { return (c >= 0x0a && c <= 0x0d) || c == 0x85 || c == 0x2028 || c == 0x2029; }
static BOOL p_wsnl(uint32_t c) { return p_ws(c) || p_nl(c); }
/* Cased scripts: Latin Extended-A/B (pairs), Greek, Cyrillic. */
static BOOL x_upper(uint32_t c) {
    if (c >= 0x100 && c <= 0x17f) return !(c & 1) ^ (c >= 0x139 && c <= 0x148) ^ (c >= 0x179);
    return (c >= 0x391 && c <= 0x3a9) || (c >= 0x400 && c <= 0x42f) || (c >= 0x1e00 && c <= 0x1eff && !(c & 1));
}
static BOOL x_lower(uint32_t c) {
    if (c >= 0x100 && c <= 0x17f) return !x_upper(c);
    return (c >= 0x3ac && c <= 0x3ce) || (c >= 0x430 && c <= 0x45f) || (c >= 0x1e00 && c <= 0x1eff && (c & 1));
}
static BOOL x_letter(uint32_t c) {
    return (c >= 0x100 && c < 0x2c0) || (c >= 0x370 && c <= 0x3ff && c != 0x37e && c != 0x387) || (c >= 0x400 && c <= 0x52f && !(c >= 0x482 && c <= 0x489))
        || (c >= 0x5d0 && c <= 0x5ea) || (c >= 0x620 && c <= 0x64a) || (c >= 0x1e00 && c <= 0x1fff) || (c >= 0x3041 && c <= 0x30ff && c != 0x30fb)
        || (c >= 0x3400 && c <= 0x4dbf) || (c >= 0x4e00 && c <= 0x9fff) || (c >= 0xac00 && c <= 0xd7a3) || (c >= 0xff21 && c <= 0xff3a) || (c >= 0xff41 && c <= 0xff5a);
}
static BOOL p_digit(uint32_t c) { return (c >= '0' && c <= '9') || (c >= 0x660 && c <= 0x669) || (c >= 0x6f0 && c <= 0x6f9) || (c >= 0x966 && c <= 0x96f) || (c >= 0xff10 && c <= 0xff19); }
static BOOL p_letter(uint32_t c) { return c < 0x80 ? ((c | 32) >= 'a' && (c | 32) <= 'z') : c < 0x100 ? (c >= 0xc0 && c != 0xd7 && c != 0xf7) || c == 0xaa || c == 0xb5 || c == 0xba : x_letter(c); }
static BOOL p_lower(uint32_t c) { return c < 0x80 ? c >= 'a' && c <= 'z' : c < 0x100 ? (c >= 0xdf && c != 0xf7) || c == 0xb5 : x_lower(c); }
static BOOL p_upper(uint32_t c) { return c < 0x80 ? c >= 'A' && c <= 'Z' : c < 0x100 ? c >= 0xc0 && c <= 0xde && c != 0xd7 : x_upper(c); }
static BOOL p_alnum(uint32_t c) { return p_letter(c) || p_digit(c); }
static BOOL p_punct(uint32_t c) { return c < 0x80 ? (c > 0x20 && c < 0x7f && !p_alnum(c) && !strchr("$+<=>^`|~", (int)c)) : (c >= 0x2010 && c <= 0x2027) || c == 0xa1 || c == 0xa7 || c == 0xab || c == 0xbb || c == 0xbf; }
static BOOL p_symbol(uint32_t c) { return c < 0x80 ? c && strchr("$+<=>^`|~", (int)c) != 0 : c == 0xa2 || c == 0xa3 || c == 0xa5 || c == 0xa9 || c == 0xae || c == 0xb0 || c == 0xb1 || c == 0xd7 || c == 0xf7 || (c >= 0x20a0 && c <= 0x20cf) || (c >= 0x2190 && c <= 0x23ff); }
static BOOL p_ctrl(uint32_t c) { return c < 0x20 || (c >= 0x7f && c < 0xa0); }
static BOOL p_alnum_ascii(uint32_t c) { return c < 0x80 && (((c | 32) >= 'a' && (c | 32) <= 'z') || (c >= '0' && c <= '9')); }
/* RFC 3986 per-component allowed sets, as Foundation defines them. */
static BOOL p_url_base(uint32_t c) { return p_alnum_ascii(c) || (c && strchr("-._~!$&'()*+,;=", (int)c)); }
static BOOL p_url_query(uint32_t c) { return p_url_base(c) || c == ':' || c == '@' || c == '/' || c == '?'; }
static BOOL p_url_path(uint32_t c) { return p_url_base(c) || c == ':' || c == '@' || c == '/'; }
static BOOL p_url_host(uint32_t c) { return p_url_base(c) || c == ':' || c == '[' || c == ']'; }
static BOOL p_url_user(uint32_t c) { return p_url_base(c); }
static BOOL p_url_pass(uint32_t c) { return p_url_base(c); }

@implementation NSCharacterSet
#define PREDEF(sel, pred, astral) + (id)sel { static NSCharacterSet *s; return predefined(&s, pred, astral); }
PREDEF(whitespaceCharacterSet, p_ws, NO)
PREDEF(whitespaceAndNewlineCharacterSet, p_wsnl, NO)
PREDEF(newlineCharacterSet, p_nl, NO)
PREDEF(decimalDigitCharacterSet, p_digit, NO)
PREDEF(letterCharacterSet, p_letter, YES)
PREDEF(lowercaseLetterCharacterSet, p_lower, NO)
PREDEF(uppercaseLetterCharacterSet, p_upper, NO)
PREDEF(alphanumericCharacterSet, p_alnum, YES)
PREDEF(punctuationCharacterSet, p_punct, NO)
PREDEF(symbolCharacterSet, p_symbol, NO)
PREDEF(controlCharacterSet, p_ctrl, NO)
PREDEF(URLQueryAllowedCharacterSet, p_url_query, NO)
PREDEF(URLPathAllowedCharacterSet, p_url_path, NO)
PREDEF(URLHostAllowedCharacterSet, p_url_host, NO)
PREDEF(URLFragmentAllowedCharacterSet, p_url_query, NO)
PREDEF(URLUserAllowedCharacterSet, p_url_user, NO)
PREDEF(URLPasswordAllowedCharacterSet, p_url_pass, NO)
+ (id)capitalizedLetterCharacterSet { return [self uppercaseLetterCharacterSet]; }
+ (id)characterSetWithCharactersInString:(NSString *)str {
    NSCharacterSet *s = make(self);
    NSUInteger n = [str length];
    for (NSUInteger i = 0; i < n; i++) {
        unichar c = [str characterAtIndex:i];
        if (c >= 0xd800 && c < 0xdc00 && i + 1 < n) { put(s, 0x10000, YES); i++; continue; }
        put(s, c, YES);
    }
    return [s autorelease];
}
+ (id)characterSetWithRange:(NSRange)r {
    NSCharacterSet *s = make(self);
    for (NSUInteger c = r.location; c < r.location + r.length; c++) put(s, (uint32_t)c, YES);
    return [s autorelease];
}
- (BOOL)characterIsMember:(unichar)c { return has(self, c); }
- (BOOL)longCharacterIsMember:(uint32_t)c { return has(self, c); }
- (BOOL)isSupersetOfSet:(NSCharacterSet *)o {
    for (int i = 0; i < BMP_BYTES; i++) if (o->_bits[i] & ~_bits[i]) return NO;
    return _astral || !o->_astral;
}
- (BOOL)hasMemberInPlane:(uint8_t)p {
    if (p) return _astral;
    for (int i = 0; i < BMP_BYTES; i++) if (_bits[i]) return YES;
    return NO;
}
- (id)invertedSet {
    NSCharacterSet *s = make([NSCharacterSet class]);
    for (int i = 0; i < BMP_BYTES; i++) s->_bits[i] = ~_bits[i];
    s->_astral = !_astral;
    return [s autorelease];
}
- (id)copyWithZone:(NSZone *)z {
    (void)z;
    NSCharacterSet *s = make([NSCharacterSet class]);
    memcpy(s->_bits, _bits, BMP_BYTES);
    s->_astral = _astral;
    return s;
}
- (id)mutableCopyWithZone:(NSZone *)z {
    (void)z;
    NSCharacterSet *s = make([NSMutableCharacterSet class]);
    memcpy(s->_bits, _bits, BMP_BYTES);
    s->_astral = _astral;
    return s;
}
- (BOOL)isEqual:(id)o {
    if (o == self) return YES;
    if (![o isKindOfClass:[NSCharacterSet class]]) return NO;
    NSCharacterSet *x = o;
    return x->_astral == _astral && memcmp(x->_bits, _bits, BMP_BYTES) == 0;
}
- (NSUInteger)hash { NSUInteger h = _astral; for (int i = 0; i < BMP_BYTES; i += 8) h = h * 31 + *(uint64_t *)(_bits + i); return h; }
- (id)description { return [NSString stringWithFormat:CFSTR("<%s: %p>"), object_getClassName(self), self]; }
- (void)dealloc { free(_bits); [super dealloc]; }
@end

@implementation NSMutableCharacterSet
+ (id)characterSet { return [make(self) autorelease]; }
- (id)init { if (!_bits) _bits = calloc(1, BMP_BYTES); return self; }
- (void)addCharactersInRange:(NSRange)r { for (NSUInteger c = r.location; c < r.location + r.length; c++) put(self, (uint32_t)c, YES); }
- (void)removeCharactersInRange:(NSRange)r { for (NSUInteger c = r.location; c < r.location + r.length; c++) put(self, (uint32_t)c, NO); }
- (void)addCharactersInString:(NSString *)s { for (NSUInteger i = 0, n = [s length]; i < n; i++) put(self, [s characterAtIndex:i], YES); }
- (void)removeCharactersInString:(NSString *)s { for (NSUInteger i = 0, n = [s length]; i < n; i++) put(self, [s characterAtIndex:i], NO); }
- (void)formUnionWithCharacterSet:(NSCharacterSet *)o { for (int i = 0; i < BMP_BYTES; i++) _bits[i] |= o->_bits[i]; _astral |= o->_astral; }
- (void)formIntersectionWithCharacterSet:(NSCharacterSet *)o { for (int i = 0; i < BMP_BYTES; i++) _bits[i] &= o->_bits[i]; _astral &= o->_astral; }
- (void)invert { for (int i = 0; i < BMP_BYTES; i++) _bits[i] = ~_bits[i]; _astral = !_astral; }
@end

/* ---- NSString methods taking a character set ---- */
enum { CaseInsensitive = 1, Backwards = 4, Anchored = 8 };

@implementation NSString (PrismCharacterSet)
- (NSRange)rangeOfCharacterFromSet:(NSCharacterSet *)set options:(NSUInteger)opts range:(NSRange)r {
    if (r.length == 0) return (NSRange){NSNotFound, 0};
    BOOL back = (opts & Backwards) != 0, anch = (opts & Anchored) != 0;
    for (NSUInteger k = 0; k < r.length; k++) {
        NSUInteger i = back ? r.location + r.length - 1 - k : r.location + k;
        unichar c = [self characterAtIndex:i];
        BOOL hit = has(set, c);
        if (!hit && (opts & CaseInsensitive) && c < 0x80) hit = has(set, (c >= 'a' && c <= 'z') ? c - 32 : (c >= 'A' && c <= 'Z') ? c + 32 : c);
        if (hit) return (NSRange){i, 1};
        if (anch) break;
    }
    return (NSRange){NSNotFound, 0};
}
- (NSRange)rangeOfCharacterFromSet:(NSCharacterSet *)set options:(NSUInteger)opts { return [self rangeOfCharacterFromSet:set options:opts range:(NSRange){0, [self length]}]; }
- (NSRange)rangeOfCharacterFromSet:(NSCharacterSet *)set { return [self rangeOfCharacterFromSet:set options:0]; }
- (id)stringByTrimmingCharactersInSet:(NSCharacterSet *)set {
    NSUInteger n = [self length], a = 0, b = n;
    while (a < b && has(set, [self characterAtIndex:a])) a++;
    while (b > a && has(set, [self characterAtIndex:b - 1])) b--;
    return [self substringWithRange:(NSRange){a, b - a}];
}
- (id)componentsSeparatedByCharactersInSet:(NSCharacterSet *)set {
    NSMutableArray *out = [NSMutableArray array];
    NSUInteger n = [self length], start = 0;
    for (NSUInteger i = 0; i <= n; i++) {
        if (i == n || has(set, [self characterAtIndex:i])) {
            [out addObject:[self substringWithRange:(NSRange){start, i - start}]];
            start = i + 1;
        }
    }
    return out;
}
- (id)stringByAddingPercentEncodingWithAllowedCharacters:(NSCharacterSet *)set {
    const unsigned char *u = (const unsigned char *)[self UTF8String];
    size_t n = strlen((const char *)u);
    char *buf = malloc(n * 3 + 1), *p = buf;
    static const char hex[] = "0123456789ABCDEF";
    for (size_t i = 0; i < n; i++) {
        if (u[i] < 0x80 && has(set, u[i])) *p++ = u[i];
        else { *p++ = '%'; *p++ = hex[u[i] >> 4]; *p++ = hex[u[i] & 15]; }
    }
    *p = 0;
    id s = [NSString stringWithUTF8String:buf];
    free(buf);
    return s;
}
@end

/* ---- NSScanner ---- */
@interface NSScanner : NSObject { @public id _string; NSUInteger _loc; NSCharacterSet *_skip; BOOL _case; id _locale; }
@end

@implementation NSScanner
+ (id)scannerWithString:(NSString *)s { return [[[self alloc] initWithString:s] autorelease]; }
+ (id)localizedScannerWithString:(NSString *)s { return [self scannerWithString:s]; }
- (id)initWithString:(NSString *)s {
    _string = [s copy];
    _skip = [[NSCharacterSet whitespaceAndNewlineCharacterSet] retain];
    return self;
}
- (void)dealloc { [_string release]; [_skip release]; [_locale release]; [super dealloc]; }
- (id)string { return _string; }
- (NSUInteger)scanLocation { return _loc; }
- (void)setScanLocation:(NSUInteger)l {
    if (l > [_string length]) _prism_raise("NSRangeException", "*** -[NSScanner setScanLocation:]: Range or index out of bounds");
    _loc = l;
}
- (NSUInteger)currentIndex { return _loc; }
- (void)setCurrentIndex:(NSUInteger)l { [self setScanLocation:l]; }
- (id)charactersToBeSkipped { return _skip; }
- (void)setCharactersToBeSkipped:(NSCharacterSet *)s { [s retain]; [_skip release]; _skip = s; }
- (BOOL)caseSensitive { return _case; }
- (void)setCaseSensitive:(BOOL)c { _case = c; }
- (id)locale { return _locale; }
- (void)setLocale:(id)l { [l retain]; [_locale release]; _locale = l; }
- (id)copyWithZone:(NSZone *)z {
    (void)z;
    NSScanner *s = [[NSScanner alloc] initWithString:_string];
    s->_loc = _loc; s->_case = _case;
    [s setCharactersToBeSkipped:_skip];
    return s;
}

static void skip(NSScanner *s) {
    NSUInteger n = [s->_string length];
    if (s->_skip) while (s->_loc < n && has(s->_skip, [s->_string characterAtIndex:s->_loc])) s->_loc++;
}
static unichar at(NSScanner *s, NSUInteger i) { return i < [s->_string length] ? [s->_string characterAtIndex:i] : 0; }

- (BOOL)isAtEnd {
    NSUInteger save = _loc;
    skip(self);
    BOOL end = _loc >= [_string length];
    _loc = save;
    return end;
}

/* The digits at the scan location (after skipping) as a C string, for strto*. */
static NSUInteger numeric(NSScanner *s, char *buf, size_t cap, BOOL hex, BOOL fp) {
    skip(s);
    NSUInteger i = s->_loc, n = [s->_string length];
    size_t k = 0;
    if (i < n && (at(s, i) == '-' || at(s, i) == '+') && !hex) buf[k++] = (char)at(s, i++);
    if (hex && at(s, i) == '0' && (at(s, i + 1) | 32) == 'x') { buf[k++] = '0'; buf[k++] = 'x'; i += 2; }
    for (; i < n && k + 1 < cap; i++) {
        unichar c = at(s, i);
        BOOL ok = (c >= '0' && c <= '9') || (hex && ((c | 32) >= 'a' && (c | 32) <= 'f'));
        if (!ok && fp && (c == '.' || ((c | 32) == 'e' && k > 0) || ((c == '-' || c == '+') && k > 0 && (buf[k - 1] | 32) == 'e'))) ok = YES;
        if (!ok) break;
        buf[k++] = (char)c;
    }
    buf[k] = 0;
    return i;
}

static BOOL scan_ll(NSScanner *s, long long *out, BOOL hex) {
    NSUInteger save = s->_loc;
    char buf[96];
    numeric(s, buf, sizeof buf, hex, NO);
    char *end;
    extern int *__error(void);
    *__error() = 0;
    long long v = hex ? (long long)strtoull(buf, &end, 16) : strtoll(buf, &end, 10);
    if (end == buf || (hex && end == buf + 2 && buf[1] == 'x')) { s->_loc = save; return NO; }
    /* numeric() left _loc untouched: skip, then advance by what strto* consumed. */
    skip(s);
    s->_loc += (NSUInteger)(end - buf);
    if (*__error()) v = hex ? -1LL : (buf[0] == '-' ? (-0x7fffffffffffffffLL - 1) : 0x7fffffffffffffffLL);
    if (out) *out = v;
    return YES;
}

- (BOOL)scanLongLong:(long long *)v { return scan_ll(self, v, NO); }
- (BOOL)scanInteger:(NSInteger *)v { long long x; if (!scan_ll(self, &x, NO)) return NO; if (v) *v = (NSInteger)x; return YES; }
- (BOOL)scanInt:(int *)v {
    long long x;
    if (!scan_ll(self, &x, NO)) return NO;
    if (v) *v = x > 0x7fffffff ? 0x7fffffff : x < -0x7fffffffLL - 1 ? (int)(-0x7fffffffLL - 1) : (int)x;
    return YES;
}
- (BOOL)scanUnsignedLongLong:(unsigned long long *)v { long long x; if (!scan_ll(self, &x, NO)) return NO; if (v) *v = (unsigned long long)x; return YES; }
- (BOOL)scanHexLongLong:(unsigned long long *)v { long long x; if (!scan_ll(self, &x, YES)) return NO; if (v) *v = (unsigned long long)x; return YES; }
- (BOOL)scanHexInt:(unsigned *)v { unsigned long long x; if (![self scanHexLongLong:&x]) return NO; if (v) *v = x > 0xffffffffULL ? 0xffffffffU : (unsigned)x; return YES; }
- (BOOL)scanDouble:(double *)v {
    NSUInteger save = _loc;
    char buf[128];
    numeric(self, buf, sizeof buf, NO, YES);
    char *end;
    double d = strtod(buf, &end);
    if (end == buf) { _loc = save; return NO; }
    skip(self);
    _loc += (NSUInteger)(end - buf);
    if (v) *v = d;
    return YES;
}
- (BOOL)scanFloat:(float *)v { double d; if (![self scanDouble:&d]) return NO; if (v) *v = (float)d; return YES; }
- (BOOL)scanDecimal:(void *)v { (void)v; double d; return [self scanDouble:&d]; }

static BOOL eqc(NSScanner *s, unichar a, unichar b) {
    if (a == b) return YES;
    if (s->_case) return NO;
    if (a < 0x80 && b < 0x80) return ((a | 32) == (b | 32)) && (a | 32) >= 'a' && (a | 32) <= 'z';
    return NO;
}

- (BOOL)scanString:(NSString *)str intoString:(NSString **)out {
    NSUInteger save = _loc;
    skip(self);
    NSUInteger m = [str length], n = [_string length];
    if (m == 0 || _loc + m > n) { _loc = save; return NO; }
    for (NSUInteger i = 0; i < m; i++)
        if (!eqc(self, [_string characterAtIndex:_loc + i], [str characterAtIndex:i])) { _loc = save; return NO; }
    if (out) *out = [_string substringWithRange:(NSRange){_loc, m}];
    _loc += m;
    return YES;
}
- (BOOL)scanCharactersFromSet:(NSCharacterSet *)set intoString:(NSString **)out {
    NSUInteger save = _loc;
    skip(self);
    NSUInteger start = _loc, n = [_string length];
    while (_loc < n && has(set, [_string characterAtIndex:_loc])) _loc++;
    if (_loc == start) { _loc = save; return NO; }
    if (out) *out = [_string substringWithRange:(NSRange){start, _loc - start}];
    return YES;
}
- (BOOL)scanUpToCharactersFromSet:(NSCharacterSet *)set intoString:(NSString **)out {
    NSUInteger save = _loc;
    skip(self);
    NSUInteger start = _loc, n = [_string length];
    while (_loc < n && !has(set, [_string characterAtIndex:_loc])) _loc++;
    if (_loc == start) { _loc = save; return NO; }
    if (out) *out = [_string substringWithRange:(NSRange){start, _loc - start}];
    return YES;
}
- (BOOL)scanUpToString:(NSString *)str intoString:(NSString **)out {
    NSUInteger save = _loc;
    skip(self);
    NSUInteger start = _loc, n = [_string length], m = [str length];
    for (; _loc < n; _loc++) {
        if (_loc + m > n) { _loc = n; break; }
        NSUInteger i = 0;
        while (i < m && eqc(self, [_string characterAtIndex:_loc + i], [str characterAtIndex:i])) i++;
        if (m && i == m) break;
    }
    if (_loc == start) { _loc = save; return NO; }
    if (out) *out = [_string substringWithRange:(NSRange){start, _loc - start}];
    return YES;
}
- (id)description { return [NSString stringWithFormat:CFSTR("<NSScanner: %p> loc=%lu"), self, (unsigned long)_loc]; }
@end
