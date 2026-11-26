/* NSJSONSerialization (RFC 8259 reader and writer, Foundation's options and output format),
 * NSNumberFormatter (no-style, decimal, percent, currency, scientific; en_US conventions),
 * NSDateFormatter (Unicode date patterns, fixed GMT offsets) and NSISO8601DateFormatter. */
#include "foundation.h"

void *malloc(size_t);
void *realloc(void *, size_t);
void free(void *);
double strtod(const char *, char **);
long long strtoll(const char *, char **, int);
int snprintf(char *, size_t, const char *, ...);
extern id const NSCocoaErrorDomain;
extern struct cfobj __kCFBooleanTrue, __kCFBooleanFalse;
double CFAbsoluteTimeGetCurrent(void);
#define EPOCH_2001 978307200.0

@interface NSObject (PrismJSONUses)
+ (id)array;
+ (id)dictionary;
+ (id)null;
+ (id)numberWithBool:(BOOL)b;
+ (id)numberWithLongLong:(long long)v;
+ (id)numberWithDouble:(double)v;
+ (id)stringWithCharacters:(const unichar *)c length:(NSUInteger)n;
+ (id)stringWithUTF8String:(const char *)s;
+ (id)dateWithTimeIntervalSinceReferenceDate:(double)t;
+ (id)timeZoneForSecondsFromGMT:(NSInteger)s;
- (void)addObject:(id)o;
- (id)objectForKey:(id)k;
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (id)bytes;
- (NSUInteger)length;
- (double)timeIntervalSinceReferenceDate;
- (id)substringWithRange:(NSRange)r;
- (NSInteger)secondsFromGMT;
@end

/* ------------------------------------------------------------------ growable buffer */
struct buf { char *p; size_t n, cap; };
static void put(struct buf *b, const char *s, size_t n) {
    if (b->n + n + 1 > b->cap) { b->cap = (b->n + n + 1) * 2; b->p = realloc(b->p, b->cap); }
    memcpy(b->p + b->n, s, n);
    b->n += n;
    b->p[b->n] = 0;
}
static void puts_(struct buf *b, const char *s) { put(b, s, strlen(s)); }
static void putc_(struct buf *b, char c) { put(b, &c, 1); }

static id json_error(id *err, const char *why, size_t at) {
    if (err) {
        char msg[200];
        snprintf(msg, sizeof msg, "%s around line 1, column %lu.", why, (unsigned long)at);
        *err = [NSError errorWithDomain:NSCocoaErrorDomain code:3840 userInfo:[NSDictionary dictionaryWithObject:[NSString stringWithUTF8String:msg] forKey:CFSTR("NSDebugDescription")]];
    }
    return nil;
}

/* ------------------------------------------------------------------ JSON reader */
enum { R_MUTABLE_CONTAINERS = 1, R_MUTABLE_LEAVES = 2, R_FRAGMENTS = 4, W_PRETTY = 1, W_SORTED = 2, W_FRAGMENTS = 4, W_NO_SLASH_ESCAPE = 8 };
struct rd { const unsigned char *s; size_t n, i; NSUInteger opts; const char *err; int depth; };

static void ws(struct rd *r) { while (r->i < r->n && (r->s[r->i] == ' ' || r->s[r->i] == '\t' || r->s[r->i] == '\n' || r->s[r->i] == '\r')) r->i++; }
static int hex4(struct rd *r, unsigned *out) {
    if (r->i + 4 > r->n) return 0;
    unsigned v = 0;
    for (int k = 0; k < 4; k++) {
        unsigned char c = r->s[r->i + k];
        v <<= 4;
        if (c >= '0' && c <= '9') v |= c - '0';
        else if ((c | 32) >= 'a' && (c | 32) <= 'f') v |= (c | 32) - 'a' + 10;
        else return 0;
    }
    r->i += 4;
    *out = v;
    return 1;
}
static id read_value(struct rd *r);
static id read_string(struct rd *r) {
    r->i++; /* opening quote */
    size_t cap = 32, n = 0;
    unichar *u = malloc(cap * sizeof *u);
    for (;;) {
        if (r->i >= r->n) { free(u); r->err = "Unterminated string"; return nil; }
        unsigned c = r->s[r->i++];
        if (n + 2 > cap) { cap *= 2; u = realloc(u, cap * sizeof *u); }
        if (c == '"') break;
        if (c < 0x20) { free(u); r->err = "Unescaped control character"; return nil; }
        if (c == '\\') {
            if (r->i >= r->n) { free(u); r->err = "Unterminated string"; return nil; }
            unsigned e = r->s[r->i++], cp;
            switch (e) {
            case '"': u[n++] = '"'; break;
            case '\\': u[n++] = '\\'; break;
            case '/': u[n++] = '/'; break;
            case 'b': u[n++] = 8; break;
            case 'f': u[n++] = 12; break;
            case 'n': u[n++] = 10; break;
            case 'r': u[n++] = 13; break;
            case 't': u[n++] = 9; break;
            case 'u':
                if (!hex4(r, &cp)) { free(u); r->err = "Invalid \\u escape"; return nil; }
                u[n++] = (unichar)cp; /* surrogate halves pass through as UTF-16 units */
                break;
            default: free(u); r->err = "Invalid escape sequence"; return nil;
            }
            continue;
        }
        if (c < 0x80) { u[n++] = (unichar)c; continue; }
        /* UTF-8 sequence -> UTF-16 */
        int extra = c >= 0xf0 ? 3 : c >= 0xe0 ? 2 : c >= 0xc0 ? 1 : -1;
        if (extra < 0 || r->i + extra > r->n) { free(u); r->err = "Invalid UTF-8"; return nil; }
        unsigned cpv = c & (0x3f >> extra);
        for (int k = 0; k < extra; k++) {
            unsigned cc = r->s[r->i++];
            if ((cc & 0xc0) != 0x80) { free(u); r->err = "Invalid UTF-8"; return nil; }
            cpv = cpv << 6 | (cc & 0x3f);
        }
        if (cpv >= 0x10000) { cpv -= 0x10000; u[n++] = (unichar)(0xd800 + (cpv >> 10)); u[n++] = (unichar)(0xdc00 + (cpv & 0x3ff)); }
        else u[n++] = (unichar)cpv;
    }
    id s = [NSString stringWithCharacters:u length:n];
    free(u);
    return (r->opts & R_MUTABLE_LEAVES) ? [[s mutableCopy] autorelease] : s;
}
static id read_number(struct rd *r) {
    size_t st = r->i;
    BOOL fp = NO;
    if (r->s[r->i] == '-') r->i++;
    if (r->i >= r->n || r->s[r->i] < '0' || r->s[r->i] > '9') { r->err = "Invalid number"; return nil; }
    if (r->s[r->i] == '0' && r->i + 1 < r->n && r->s[r->i + 1] >= '0' && r->s[r->i + 1] <= '9') { r->err = "Number with leading zero"; return nil; }
    while (r->i < r->n && r->s[r->i] >= '0' && r->s[r->i] <= '9') r->i++;
    if (r->i < r->n && r->s[r->i] == '.') {
        fp = YES; r->i++;
        if (r->i >= r->n || r->s[r->i] < '0' || r->s[r->i] > '9') { r->err = "Invalid number"; return nil; }
        while (r->i < r->n && r->s[r->i] >= '0' && r->s[r->i] <= '9') r->i++;
    }
    if (r->i < r->n && (r->s[r->i] | 32) == 'e') {
        fp = YES; r->i++;
        if (r->i < r->n && (r->s[r->i] == '+' || r->s[r->i] == '-')) r->i++;
        if (r->i >= r->n || r->s[r->i] < '0' || r->s[r->i] > '9') { r->err = "Invalid number"; return nil; }
        while (r->i < r->n && r->s[r->i] >= '0' && r->s[r->i] <= '9') r->i++;
    }
    char tmp[64];
    size_t len = r->i - st;
    if (len >= sizeof tmp) fp = YES, len = sizeof tmp - 1;
    memcpy(tmp, r->s + st, len);
    tmp[len] = 0;
    if (!fp) {
        extern int *__error(void);
        *__error() = 0;
        long long v = strtoll(tmp, 0, 10);
        if (!*__error()) return [NSNumber numberWithLongLong:v];
    }
    return [NSNumber numberWithDouble:strtod(tmp, 0)];
}
static BOOL lit(struct rd *r, const char *w) {
    size_t k = strlen(w);
    if (r->i + k > r->n || memcmp(r->s + r->i, w, k)) return NO;
    r->i += k;
    return YES;
}
static id read_value(struct rd *r) {
    ws(r);
    if (r->i >= r->n) { r->err = "Unexpected end of file"; return nil; }
    if (++r->depth > 512) { r->err = "Too many nested arrays or dictionaries"; return nil; }
    id result = nil;
    unsigned char c = r->s[r->i];
    BOOL mut = (r->opts & R_MUTABLE_CONTAINERS) != 0;
    if (c == '{') {
        r->i++;
        id d = [NSMutableDictionary dictionary];
        ws(r);
        if (r->i < r->n && r->s[r->i] == '}') { r->i++; result = d; }
        else for (;;) {
            ws(r);
            if (r->i >= r->n || r->s[r->i] != '"') { r->err = "No string key for value in object"; return nil; }
            id k = read_string(r);
            if (!k) return nil;
            ws(r);
            if (r->i >= r->n || r->s[r->i] != ':') { r->err = "No ':' after key in object"; return nil; }
            r->i++;
            id v = read_value(r);
            if (!v) return nil;
            [d setObject:v forKey:k];
            ws(r);
            if (r->i < r->n && r->s[r->i] == ',') { r->i++; continue; }
            if (r->i < r->n && r->s[r->i] == '}') { r->i++; result = d; break; }
            r->err = "Badly formed object";
            return nil;
        }
        if (!mut) result = [[result copy] autorelease];
    } else if (c == '[') {
        r->i++;
        id a = [NSMutableArray array];
        ws(r);
        if (r->i < r->n && r->s[r->i] == ']') { r->i++; result = a; }
        else for (;;) {
            id v = read_value(r);
            if (!v) return nil;
            [a addObject:v];
            ws(r);
            if (r->i < r->n && r->s[r->i] == ',') { r->i++; continue; }
            if (r->i < r->n && r->s[r->i] == ']') { r->i++; result = a; break; }
            r->err = "Badly formed array";
            return nil;
        }
        if (!mut) result = [[result copy] autorelease];
    } else if (c == '"') result = read_string(r);
    else if (c == '-' || (c >= '0' && c <= '9')) result = read_number(r);
    else if (lit(r, "true")) result = (id)&__kCFBooleanTrue;
    else if (lit(r, "false")) result = (id)&__kCFBooleanFalse;
    else if (lit(r, "null")) result = [NSNull null];
    else r->err = "Invalid value";
    r->depth--;
    return result;
}

/* ------------------------------------------------------------------ JSON writer */
static BOOL write_value(struct buf *b, id o, NSUInteger opts, int indent, id *err);
static void write_string(struct buf *b, id s, NSUInteger opts) {
    const unsigned char *u = (const unsigned char *)[s UTF8String];
    putc_(b, '"');
    for (; *u; u++) {
        unsigned char c = *u;
        char esc[8];
        if (c == '"') puts_(b, "\\\"");
        else if (c == '\\') puts_(b, "\\\\");
        else if (c == '/' && !(opts & W_NO_SLASH_ESCAPE)) puts_(b, "\\/");
        else if (c == '\n') puts_(b, "\\n");
        else if (c == '\r') puts_(b, "\\r");
        else if (c == '\t') puts_(b, "\\t");
        else if (c == 8) puts_(b, "\\b");
        else if (c == 12) puts_(b, "\\f");
        else if (c < 0x20) { snprintf(esc, sizeof esc, "\\u%04x", c); puts_(b, esc); }
        else putc_(b, (char)c);
    }
    putc_(b, '"');
}
static void write_number(struct buf *b, id n) {
    char t[64];
    if (n == (id)&__kCFBooleanTrue) { puts_(b, "true"); return; }
    if (n == (id)&__kCFBooleanFalse) { puts_(b, "false"); return; }
    const char *ty = [n objCType];
    if (*ty == 'f' || *ty == 'd') {
        double d = [n doubleValue];
        /* The shortest form that reads back as the same double. */
        for (int p = 1; p <= 17; p++) {
            snprintf(t, sizeof t, "%.*g", p, d);
            if (strtod(t, 0) == d) break;
        }
    } else snprintf(t, sizeof t, "%lld", [n longLongValue]);
    puts_(b, t);
}
static void newline(struct buf *b, NSUInteger opts, int indent) {
    if (!(opts & W_PRETTY)) return;
    putc_(b, '\n');
    for (int k = 0; k < indent; k++) puts_(b, "  ");
}
static BOOL write_value(struct buf *b, id o, NSUInteger opts, int indent, id *err) {
    if (cf_is(o, [NSString class])) { write_string(b, o, opts); return YES; }
    if (cf_is(o, [NSNumber class])) {
        double d = [o doubleValue];
        if (d != d || d - d != 0) { json_error(err, "Invalid number value (NaN or infinity) in JSON write", 0); return NO; }
        write_number(b, o);
        return YES;
    }
    if (o == [NSNull null]) { puts_(b, "null"); return YES; }
    if (cf_is(o, [NSArray class])) {
        NSUInteger n = [o count];
        putc_(b, '[');
        for (NSUInteger i = 0; i < n; i++) {
            if (i) putc_(b, ',');
            newline(b, opts, indent + 1);
            if (!write_value(b, [o objectAtIndex:i], opts, indent + 1, err)) return NO;
        }
        if (n) newline(b, opts, indent);
        putc_(b, ']');
        return YES;
    }
    if (cf_is(o, [NSDictionary class])) {
        id keys = [o allKeys];
        if (opts & W_SORTED) keys = [keys sortedArrayUsingSelector:@selector(compare:)];
        NSUInteger n = [keys count];
        putc_(b, '{');
        for (NSUInteger i = 0; i < n; i++) {
            id k = [keys objectAtIndex:i];
            if (!cf_is(k, [NSString class])) { json_error(err, "Invalid (non-string) key in JSON dictionary", 0); return NO; }
            if (i) putc_(b, ',');
            newline(b, opts, indent + 1);
            write_string(b, k, opts);
            puts_(b, (opts & W_PRETTY) ? " : " : ":");
            if (!write_value(b, [o objectForKey:k], opts, indent + 1, err)) return NO;
        }
        if (n) newline(b, opts, indent);
        putc_(b, '}');
        return YES;
    }
    json_error(err, "Invalid type in JSON write", 0);
    return NO;
}

@interface NSJSONSerialization : NSObject
@end
@implementation NSJSONSerialization
+ (id)JSONObjectWithData:(id)data options:(NSUInteger)opts error:(id *)err {
    if (!data) _prism_raise("NSInvalidArgumentException", "data parameter is nil");
    struct rd r = {(const unsigned char *)[data bytes], [data length], 0, opts, 0, 0};
    if (r.n >= 3 && r.s[0] == 0xef && r.s[1] == 0xbb && r.s[2] == 0xbf) r.i = 3; /* BOM */
    ws(&r);
    if (r.i < r.n && !(opts & R_FRAGMENTS) && r.s[r.i] != '{' && r.s[r.i] != '[')
        return json_error(err, "JSON text did not start with array or object and option to allow fragments not set.", r.i);
    id v = read_value(&r);
    if (!v) return json_error(err, r.err ? r.err : "Invalid value", r.i);
    ws(&r);
    if (r.i != r.n) return json_error(err, "Garbage at end", r.i);
    return v;
}
+ (id)dataWithJSONObject:(id)o options:(NSUInteger)opts error:(id *)err {
    if (!(opts & W_FRAGMENTS) && !cf_is(o, [NSArray class]) && !cf_is(o, [NSDictionary class]))
        _prism_raise("NSInvalidArgumentException", "*** +[NSJSONSerialization dataWithJSONObject:options:error:]: Invalid top-level type in JSON write");
    struct buf b = {0};
    put(&b, "", 0);
    if (!write_value(&b, o, opts, 0, err)) { free(b.p); return nil; }
    id d = [NSData dataWithBytes:b.p length:b.n];
    free(b.p);
    return d;
}
static BOOL valid(id o, int depth) {
    if (depth > 512) return NO;
    if (cf_is(o, [NSString class]) || o == [NSNull null]) return YES;
    if (cf_is(o, [NSNumber class])) { double d = [o doubleValue]; return d == d && d - d == 0; }
    if (cf_is(o, [NSArray class])) { for (NSUInteger i = 0, n = [o count]; i < n; i++) if (!valid([o objectAtIndex:i], depth + 1)) return NO; return YES; }
    if (cf_is(o, [NSDictionary class])) {
        id keys = [o allKeys];
        for (NSUInteger i = 0, n = [keys count]; i < n; i++) {
            id k = [keys objectAtIndex:i];
            if (!cf_is(k, [NSString class]) || !valid([o objectForKey:k], depth + 1)) return NO;
        }
        return YES;
    }
    return NO;
}
+ (BOOL)isValidJSONObject:(id)o { return (cf_is(o, [NSArray class]) || cf_is(o, [NSDictionary class])) && valid(o, 0); }
@end

/* ------------------------------------------------------------------ NSFormatter */
@interface NSFormatter : NSObject
@end
@implementation NSFormatter
- (id)stringForObjectValue:(id)o { (void)o; return nil; }
- (BOOL)getObjectValue:(id *)o forString:(id)s errorDescription:(id *)e { (void)o; (void)s; (void)e; return NO; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
@end

enum { NF_NONE, NF_DECIMAL, NF_CURRENCY, NF_PERCENT, NF_SCIENTIFIC, NF_SPELLOUT };
@interface NSNumberFormatter : NSFormatter {
@public
    NSUInteger _style;
    NSInteger _minFrac, _maxFrac, _minInt;
    BOOL _grouping, _setFrac, _lenient;
    id _prefix, _suffix, _currency, _decimalSep, _groupSep, _nilSym, _locale;
}
@end
@implementation NSNumberFormatter
- (id)init { _maxFrac = 0; _minInt = 1; _decimalSep = [CFSTR(".") retain]; _groupSep = [CFSTR(",") retain]; _currency = [CFSTR("$") retain]; return self; }
- (NSUInteger)numberStyle { return _style; }
- (void)setNumberStyle:(NSUInteger)s {
    _style = s;
    _grouping = s == NF_DECIMAL || s == NF_CURRENCY || s == NF_PERCENT;
    if (!_setFrac) {
        if (s == NF_DECIMAL) { _minFrac = 0; _maxFrac = 3; }
        else if (s == NF_CURRENCY) { _minFrac = 2; _maxFrac = 2; }
        else if (s == NF_SCIENTIFIC) { _minFrac = 0; _maxFrac = 6; }
        else { _minFrac = 0; _maxFrac = 0; }
    }
}
- (NSInteger)minimumFractionDigits { return _minFrac; }
- (void)setMinimumFractionDigits:(NSInteger)n { _minFrac = n; if (_maxFrac < n) _maxFrac = n; _setFrac = YES; }
- (NSInteger)maximumFractionDigits { return _maxFrac; }
- (void)setMaximumFractionDigits:(NSInteger)n { _maxFrac = n; if (_minFrac > n) _minFrac = n; _setFrac = YES; }
- (NSInteger)minimumIntegerDigits { return _minInt; }
- (void)setMinimumIntegerDigits:(NSInteger)n { _minInt = n; }
- (BOOL)usesGroupingSeparator { return _grouping; }
- (void)setUsesGroupingSeparator:(BOOL)g { _grouping = g; }
#define STRPROP(get, set, ivar) - (id)get { return ivar; } - (void)set:(id)v { id o = ivar; ivar = [v copy]; [o release]; }
STRPROP(positivePrefix, setPositivePrefix, _prefix)
STRPROP(positiveSuffix, setPositiveSuffix, _suffix)
STRPROP(currencySymbol, setCurrencySymbol, _currency)
STRPROP(decimalSeparator, setDecimalSeparator, _decimalSep)
STRPROP(groupingSeparator, setGroupingSeparator, _groupSep)
STRPROP(nilSymbol, setNilSymbol, _nilSym)
STRPROP(locale, setLocale, _locale)
- (BOOL)isLenient { return _lenient; }
- (void)setLenient:(BOOL)l { _lenient = l; }
- (void)setGeneratesDecimalNumbers:(BOOL)g { (void)g; }
- (void)setAllowsFloats:(BOOL)a { (void)a; }

/* Round half-even to `frac` digits, then format with grouping and trimmed optional zeros. */
static void format_fixed(NSNumberFormatter *f, double v, struct buf *b) {
    char raw[400];
    snprintf(raw, sizeof raw, "%.*f", (int)f->_maxFrac, v < 0 ? -v : v);
    char *dot = strchr(raw, '.');
    size_t ilen = dot ? (size_t)(dot - raw) : strlen(raw);
    const char *frac = dot ? dot + 1 : "";
    size_t flen = strlen(frac);
    while (flen > (size_t)f->_minFrac && frac[flen - 1] == '0') flen--;
    const char *gs = [f->_groupSep UTF8String], *ds = [f->_decimalSep UTF8String];
    for (NSInteger k = (NSInteger)ilen; k < f->_minInt; k++) putc_(b, '0');
    for (size_t k = 0; k < ilen; k++) {
        putc_(b, raw[k]);
        size_t left = ilen - k - 1;
        if (f->_grouping && left && left % 3 == 0) puts_(b, gs);
    }
    if (flen) { puts_(b, ds); put(b, frac, flen); }
}
- (id)stringFromNumber:(id)n {
    if (!n) return _nilSym;
    double v = [n doubleValue];
    if (v != v) return CFSTR("NaN");
    struct buf b = {0};
    put(&b, "", 0);
    BOOL neg = v < 0;
    if (neg && _style != NF_SCIENTIFIC) putc_(&b, '-');
    if (_prefix) puts_(&b, [_prefix UTF8String]);
    switch (_style) {
    case NF_CURRENCY: puts_(&b, [_currency UTF8String]); format_fixed(self, v, &b); break;
    case NF_PERCENT: format_fixed(self, v * 100, &b); putc_(&b, '%'); break;
    case NF_SCIENTIFIC: {
        char t[64];
        snprintf(t, sizeof t, "%.*E", (int)_maxFrac, v);
        /* 1.234500E+03 -> 1.2345E3 */
        char *e = strchr(t, 'E'), mant[64];
        size_t ml = (size_t)(e - t);
        memcpy(mant, t, ml); mant[ml] = 0;
        if (strchr(mant, '.')) { while (ml && mant[ml - 1] == '0') mant[--ml] = 0; if (ml && mant[ml - 1] == '.') mant[--ml] = 0; }
        puts_(&b, mant);
        snprintf(t, sizeof t, "E%d", (int)strtoll(e + 1, 0, 10));
        puts_(&b, t);
        break;
    }
    default:
        if (_style == NF_NONE && !_setFrac) { char t[64]; snprintf(t, sizeof t, "%.0f", v < 0 ? -v : v); puts_(&b, t); }
        else format_fixed(self, v, &b);
    }
    if (_suffix) puts_(&b, [_suffix UTF8String]);
    id s = [NSString stringWithUTF8String:b.p];
    free(b.p);
    return s;
}
- (id)stringForObjectValue:(id)o { return cf_is(o, [NSNumber class]) ? [self stringFromNumber:o] : nil; }
- (id)numberFromString:(id)s {
    const char *u = [s UTF8String], *gs = [_groupSep UTF8String], *ds = [_decimalSep UTF8String], *cs = [_currency UTF8String];
    size_t gl = strlen(gs), dl = strlen(ds), cl = strlen(cs);
    char t[256];
    size_t k = 0;
    BOOL digits = NO, percent = NO;
    while (*u == ' ') u++;
    if (*u == '-' || *u == '+') t[k++] = *u++;
    if (cl && !strncmp(u, cs, cl) && (_style == NF_CURRENCY || _lenient)) u += cl;
    for (; *u && k < sizeof t - 1; ) {
        if (*u >= '0' && *u <= '9') { t[k++] = *u++; digits = YES; }
        else if (gl && !strncmp(u, gs, gl) && _grouping) u += gl;
        else if (dl && !strncmp(u, ds, dl)) { t[k++] = '.'; u += dl; }
        else if ((*u == 'e' || *u == 'E') && digits) { t[k++] = 'e'; u++; if (*u == '-' || *u == '+') t[k++] = *u++; }
        else if (*u == '%' && (_style == NF_PERCENT || _lenient)) { percent = YES; u++; }
        else break;
    }
    while (*u == ' ') u++;
    if (!digits || *u) return nil;
    t[k] = 0;
    double v = strtod(t, 0);
    if (percent || _style == NF_PERCENT) v /= 100;
    if (v == (double)(long long)v && !strchr(t, '.') && !strchr(t, 'e') && !percent) return [NSNumber numberWithLongLong:(long long)v];
    return [NSNumber numberWithDouble:v];
}
- (BOOL)getObjectValue:(id *)o forString:(id)s errorDescription:(id *)e {
    id n = [self numberFromString:s];
    if (o) *o = n;
    if (!n && e) *e = CFSTR("Formatting error.");
    return n != nil;
}
+ (id)localizedStringFromNumber:(id)n numberStyle:(NSUInteger)st {
    NSNumberFormatter *f = [[self new] autorelease];
    [f setNumberStyle:st];
    return [f stringFromNumber:n];
}
- (void)dealloc { [_prefix release]; [_suffix release]; [_currency release]; [_decimalSep release]; [_groupSep release]; [_nilSym release]; [_locale release]; [super dealloc]; }
@end

/* ------------------------------------------------------------------ dates */
static const char *const months[] = {"January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"};
static const char *const days[] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};

/* Days since 1970-01-01 for a proleptic Gregorian date, and back (H. Hinnant's algorithms). */
static long days_from_civil(long y, unsigned m, unsigned d) {
    y -= m <= 2;
    long era = (y >= 0 ? y : y - 399) / 400;
    unsigned yoe = (unsigned)(y - era * 400);
    unsigned doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    unsigned doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + (long)doe - 719468;
}
struct civil { long y; int mo, d, h, mi, s, ms, wd; };
static struct civil civil_from(double unix_secs) {
    double fl = floor_(unix_secs);
    long long secs = (long long)fl;
    int ms = (int)((unix_secs - fl) * 1000 + 0.5);
    if (ms >= 1000) { ms -= 1000; secs++; }
    long long z = secs >= 0 ? secs / 86400 : (secs - 86399) / 86400;
    long long rem = secs - z * 86400;
    struct civil c;
    c.wd = (int)((z % 7 + 11) % 7); /* 1970-01-01 was a Thursday */
    z += 719468;
    long era = (long)((z >= 0 ? z : z - 146096) / 146097);
    unsigned doe = (unsigned)(z - era * 146097);
    unsigned yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    unsigned mp = (5 * doy + 2) / 153;
    c.d = (int)(doy - (153 * mp + 2) / 5 + 1);
    c.mo = (int)(mp < 10 ? mp + 3 : mp - 9);
    c.y = (long)yoe + era * 400 + (c.mo <= 2);
    c.h = (int)(rem / 3600); c.mi = (int)(rem / 60 % 60); c.s = (int)(rem % 60); c.ms = ms;
    return c;
}

@interface NSDateFormatter : NSFormatter { @public id _format, _locale; NSInteger _tz; NSUInteger _dateStyle, _timeStyle; BOOL _lenient; }
@end

static void fmt_offset(struct buf *b, NSInteger tz, int colon, int z_for_zero) {
    if (z_for_zero && tz == 0) { putc_(b, 'Z'); return; }
    char t[16];
    long a = tz < 0 ? -tz : tz;
    snprintf(t, sizeof t, colon ? "%c%02ld:%02ld" : "%c%02ld%02ld", tz < 0 ? '-' : '+', a / 3600, a / 60 % 60);
    puts_(b, t);
}
static id format_date(const char *pat, double ref, NSInteger tz) {
    struct civil c = civil_from(ref + EPOCH_2001 + (double)tz);
    struct buf b = {0};
    put(&b, "", 0);
    char t[32];
    for (const char *p = pat; *p;) {
        if (*p == '\'') {
            p++;
            if (*p == '\'') { putc_(&b, '\''); p++; continue; }
            while (*p && !(*p == '\'' && p[1] != '\'')) { if (*p == '\'') p++; putc_(&b, *p++); }
            if (*p) p++;
            continue;
        }
        if (!((*p | 32) >= 'a' && (*p | 32) <= 'z')) { putc_(&b, *p++); continue; }
        char ch = *p;
        int n = 0;
        while (*p == ch) { p++; n++; }
        switch (ch) {
        case 'y': case 'u': case 'Y':
            if (n == 2) snprintf(t, sizeof t, "%02ld", ((c.y % 100) + 100) % 100); else snprintf(t, sizeof t, "%0*ld", n, c.y);
            puts_(&b, t); break;
        case 'M': case 'L':
            if (n >= 4) puts_(&b, months[c.mo - 1]);
            else if (n == 3) put(&b, months[c.mo - 1], 3);
            else { snprintf(t, sizeof t, "%0*d", n, c.mo); puts_(&b, t); }
            break;
        case 'd': snprintf(t, sizeof t, "%0*d", n, c.d); puts_(&b, t); break;
        case 'D': {
            int doy = (int)(days_from_civil(c.y, (unsigned)c.mo, (unsigned)c.d) - days_from_civil(c.y, 1, 1)) + 1;
            snprintf(t, sizeof t, "%0*d", n, doy); puts_(&b, t); break;
        }
        case 'E': case 'e': case 'c':
            if (n >= 4) puts_(&b, days[c.wd]); else if (ch != 'E' && n <= 2) { snprintf(t, sizeof t, "%d", c.wd + 1); puts_(&b, t); } else put(&b, days[c.wd], 3);
            break;
        case 'H': snprintf(t, sizeof t, "%0*d", n, c.h); puts_(&b, t); break;
        case 'k': snprintf(t, sizeof t, "%0*d", n, c.h ? c.h : 24); puts_(&b, t); break;
        case 'h': snprintf(t, sizeof t, "%0*d", n, c.h % 12 ? c.h % 12 : 12); puts_(&b, t); break;
        case 'K': snprintf(t, sizeof t, "%0*d", n, c.h % 12); puts_(&b, t); break;
        case 'm': snprintf(t, sizeof t, "%0*d", n, c.mi); puts_(&b, t); break;
        case 's': snprintf(t, sizeof t, "%0*d", n, c.s); puts_(&b, t); break;
        case 'S': { snprintf(t, sizeof t, "%03d", c.ms); t[n < 3 ? n : 3] = 0; puts_(&b, t); for (int k = 3; k < n; k++) putc_(&b, '0'); break; }
        case 'a': puts_(&b, c.h < 12 ? "AM" : "PM"); break;
        case 'Z': if (n <= 3) fmt_offset(&b, tz, 0, 0); else if (n == 5) fmt_offset(&b, tz, 1, 1); else { puts_(&b, "GMT"); fmt_offset(&b, tz, 1, 0); } break;
        case 'X': case 'x': fmt_offset(&b, tz, n == 3 || n == 5, ch == 'X'); break;
        case 'z': case 'v': case 'V': case 'O': puts_(&b, "GMT"); if (tz) fmt_offset(&b, tz, 1, 0); break;
        case 'G': puts_(&b, c.y > 0 ? "AD" : "BC"); break;
        default: for (int k = 0; k < n; k++) putc_(&b, ch);
        }
    }
    id s = [NSString stringWithUTF8String:b.p];
    free(b.p);
    return s;
}

/* Parse `s` with `pat`. Returns 0 (and leaves *out alone) on mismatch. */
static int match_word(const char **s, const char *const *words, int count, int abbrev) {
    for (int i = 0; i < count; i++) {
        size_t l = abbrev ? 3 : strlen(words[i]);
        int ok = 1;
        for (size_t k = 0; k < l; k++) if (((*s)[k] | 32) != (words[i][k] | 32)) { ok = 0; break; }
        if (ok) { *s += l; return i + 1; }
    }
    return 0;
}
static int num(const char **s, int maxd, long *out) {
    long v = 0;
    int k = 0, neg = 0;
    if (**s == '-' && maxd > 4) { neg = 1; (*s)++; }
    while (k < maxd && **s >= '0' && **s <= '9') { v = v * 10 + (**s - '0'); (*s)++; k++; }
    *out = neg ? -v : v;
    return k > 0;
}
static int parse_date(const char *pat, const char *s, NSInteger deftz, double *out) {
    long y = 2000, mo = 1, d = 1, h = 0, mi = 0, sec = 0, ms = 0, pm = -1;
    long tz = deftz;
    for (const char *p = pat; *p;) {
        if (*p == '\'') {
            p++;
            while (*p && *p != '\'') { if (*s != *p) return 0; s++; p++; }
            if (*p) p++;
            continue;
        }
        if (!((*p | 32) >= 'a' && (*p | 32) <= 'z')) { if (*s != *p) return 0; s++; p++; continue; }
        char ch = *p;
        int n = 0;
        while (*p == ch) { p++; n++; }
        /* A field directly followed by another field reads exactly n digits. */
        int fixed = ((*p | 32) >= 'a' && (*p | 32) <= 'z');
        long v;
        switch (ch) {
        case 'y': case 'u': case 'Y':
            if (!num(&s, n == 2 ? 2 : fixed ? n : 9, &v)) return 0;
            y = n == 2 ? 2000 + v - (v >= 69 ? 100 : 0) : v; break;
        case 'M': case 'L':
            if (n >= 3) { int m = match_word(&s, (const char *const *)months, 12, n == 3); if (!m) return 0; mo = m; }
            else if (!num(&s, fixed ? n : 2, &mo)) return 0;
            break;
        case 'd': if (!num(&s, fixed ? n : 2, &d)) return 0; break;
        case 'H': case 'k': if (!num(&s, fixed ? n : 2, &h)) return 0; break;
        case 'h': case 'K': if (!num(&s, fixed ? n : 2, &h)) return 0; break;
        case 'm': if (!num(&s, fixed ? n : 2, &mi)) return 0; break;
        case 's': if (!num(&s, fixed ? n : 2, &sec)) return 0; break;
        case 'S': { const char *st = s; if (!num(&s, 9, &v)) return 0; int dg = (int)(s - st); while (dg > 3) { v /= 10; dg--; } while (dg < 3) { v *= 10; dg++; } ms = v; break; }
        case 'E': case 'e': case 'c': if (!match_word(&s, (const char *const *)days, 7, n < 4)) return 0; break;
        case 'a':
            if ((s[0] | 32) == 'a' && (s[1] | 32) == 'm') pm = 0; else if ((s[0] | 32) == 'p' && (s[1] | 32) == 'm') pm = 1; else return 0;
            s += 2; break;
        case 'Z': case 'X': case 'x': case 'z': case 'O':
            if (*s == 'Z') { tz = 0; s++; break; }
            if (!strncmp(s, "GMT", 3) || !strncmp(s, "UTC", 3)) { s += 3; tz = 0; if (*s != '+' && *s != '-') break; }
            if (*s == '+' || *s == '-') {
                int sg = *s++ == '-' ? -1 : 1;
                long hh = 0, mm = 0;
                if (!num(&s, 2, &hh)) return 0;
                if (*s == ':') s++;
                num(&s, 2, &mm);
                tz = sg * (hh * 3600 + mm * 60);
            } else return 0;
            break;
        default: return 0;
        }
    }
    if (*s) return 0;
    if (pm == 1 && h < 12) h += 12;
    if (pm == 0 && h == 12) h = 0;
    if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 24 || mi > 59 || sec > 60) return 0;
    double unix_secs = (double)days_from_civil(y, (unsigned)mo, (unsigned)d) * 86400 + h * 3600 + mi * 60 + sec + ms / 1000.0 - tz;
    *out = unix_secs - EPOCH_2001;
    return 1;
}

@implementation NSDateFormatter
- (id)init { _format = [CFSTR("") retain]; return self; }
- (id)dateFormat { return _format; }
- (void)setDateFormat:(id)f { id o = _format; _format = [f copy]; [o release]; }
- (id)locale { return _locale; }
- (void)setLocale:(id)l { id o = _locale; _locale = [l retain]; [o release]; }
- (id)timeZone { return [(id)objc_getClass("NSTimeZone") timeZoneForSecondsFromGMT:_tz]; }
- (void)setTimeZone:(id)z { _tz = z ? [z secondsFromGMT] : 0; }
- (BOOL)isLenient { return _lenient; }
- (void)setLenient:(BOOL)l { _lenient = l; }
- (NSUInteger)dateStyle { return _dateStyle; }
- (NSUInteger)timeStyle { return _timeStyle; }
/* en_US styles: short 1/2/06, medium Jan 2, 2006, long January 2, 2006, full Monday, January 2, 2006. */
- (void)restyle {
    static const char *const ds[] = {"", "M/d/yy", "MMM d, y", "MMMM d, y", "EEEE, MMMM d, y"};
    static const char *const ts[] = {"", "h:mm a", "h:mm:ss a", "h:mm:ss a z", "h:mm:ss a zzzz"};
    char f[64];
    snprintf(f, sizeof f, "%s%s%s", ds[_dateStyle & 7 ? (_dateStyle > 4 ? 4 : _dateStyle) : 0],
             _dateStyle && _timeStyle ? (_dateStyle >= 3 ? " 'at' " : ", ") : "", ts[_timeStyle > 4 ? 4 : _timeStyle]);
    [self setDateFormat:[NSString stringWithUTF8String:f]];
}
- (void)setDateStyle:(NSUInteger)s { _dateStyle = s; [self restyle]; }
- (void)setTimeStyle:(NSUInteger)s { _timeStyle = s; [self restyle]; }
- (void)setFormatterBehavior:(NSUInteger)b { (void)b; }
- (void)setCalendar:(id)c { (void)c; }
- (void)setDoesRelativeDateFormatting:(BOOL)r { (void)r; }
- (void)setLocalizedDateFormatFromTemplate:(id)t { [self setDateFormat:t]; }
+ (id)dateFormatFromTemplate:(id)t options:(NSUInteger)o locale:(id)l { (void)o; (void)l; return t; }
- (id)stringFromDate:(id)d { return d ? format_date([_format UTF8String], [d timeIntervalSinceReferenceDate], _tz) : nil; }
- (id)stringForObjectValue:(id)o { return [self stringFromDate:o]; }
- (id)dateFromString:(id)s {
    double t;
    if (!s || !parse_date([_format UTF8String], [s UTF8String], _tz, &t)) return nil;
    return [NSDate dateWithTimeIntervalSinceReferenceDate:t];
}
- (BOOL)getObjectValue:(id *)o forString:(id)s errorDescription:(id *)e {
    id d = [self dateFromString:s];
    if (o) *o = d;
    if (!d && e) *e = CFSTR("Formatting error.");
    return d != nil;
}
+ (id)localizedStringFromDate:(id)d dateStyle:(NSUInteger)ds timeStyle:(NSUInteger)ts {
    NSDateFormatter *f = [[self new] autorelease];
    f->_dateStyle = ds; f->_timeStyle = ts;
    [f restyle];
    return [f stringFromDate:d];
}
- (void)dealloc { [_format release]; [_locale release]; [super dealloc]; }
@end

/* NSISO8601DateFormatter: the default options give 2006-01-02T15:04:05Z. */
enum { ISO_YEAR = 1, ISO_MONTH = 2, ISO_WEEK = 4, ISO_DAY = 16, ISO_TIME = 32, ISO_TZ = 64, ISO_SPACE = 128, ISO_DASH = 256, ISO_COLON = 512,
       ISO_COLON_TZ = 1024, ISO_FRACTION = 2048, ISO_FULL_DATE = 275, ISO_INTERNET = 1907 };
@interface NSISO8601DateFormatter : NSFormatter { @public NSUInteger _opts; NSInteger _tz; }
@end
@implementation NSISO8601DateFormatter
- (id)init { _opts = ISO_INTERNET; return self; }
- (NSUInteger)formatOptions { return _opts; }
- (void)setFormatOptions:(NSUInteger)o { _opts = o; }
- (id)timeZone { return [(id)objc_getClass("NSTimeZone") timeZoneForSecondsFromGMT:_tz]; }
- (void)setTimeZone:(id)z { _tz = z ? [z secondsFromGMT] : 0; }
- (id)pattern {
    char f[64] = "";
    if (_opts & (ISO_YEAR | ISO_MONTH | ISO_DAY)) {
        const char *dash = (_opts & ISO_DASH) ? "-" : "";
        snprintf(f, sizeof f, "%s%s%s%s%s", _opts & ISO_YEAR ? "yyyy" : "", _opts & ISO_MONTH ? dash : "", _opts & ISO_MONTH ? "MM" : "",
                 _opts & ISO_DAY ? dash : "", _opts & ISO_DAY ? "dd" : "");
    }
    if (_opts & ISO_TIME) {
        size_t l = strlen(f);
        const char *c = (_opts & ISO_COLON) ? ":" : "";
        snprintf(f + l, sizeof f - l, "%sHH%smm%sss%s", l ? ((_opts & ISO_SPACE) ? " " : "'T'") : "", c, c, (_opts & ISO_FRACTION) ? ".SSS" : "");
    }
    if (_opts & ISO_TZ) { size_t l = strlen(f); snprintf(f + l, sizeof f - l, "%s", (_opts & ISO_COLON_TZ) ? "XXXXX" : "XXXX"); }
    return [NSString stringWithUTF8String:f];
}
- (id)stringFromDate:(id)d { return d ? format_date([[self pattern] UTF8String], [d timeIntervalSinceReferenceDate], _tz) : nil; }
- (id)stringForObjectValue:(id)o { return [self stringFromDate:o]; }
- (id)dateFromString:(id)s {
    double t;
    id pat = [self pattern];
    if (!s) return nil;
    if (parse_date([pat UTF8String], [s UTF8String], _tz, &t)) return [NSDate dateWithTimeIntervalSinceReferenceDate:t];
    /* Accept a fraction even when the options do not ask for one. */
    if (!(_opts & ISO_FRACTION)) {
        _opts |= ISO_FRACTION;
        int ok = parse_date([[self pattern] UTF8String], [s UTF8String], _tz, &t);
        _opts &= ~(NSUInteger)ISO_FRACTION;
        if (ok) return [NSDate dateWithTimeIntervalSinceReferenceDate:t];
    }
    return nil;
}
+ (id)stringFromDate:(id)d timeZone:(id)z formatOptions:(NSUInteger)o {
    NSISO8601DateFormatter *f = [[self new] autorelease];
    f->_opts = o;
    [f setTimeZone:z];
    return [f stringFromDate:d];
}
@end

/* ------------------------------------------------------------------ NSTimeZone */
/* Fixed offsets only (no tz database): UTC/GMT, "GMT+hh[:mm]" and offsets. The local zone is UTC. */
@interface NSTimeZone : NSObject { @public NSInteger _off; id _name; }
@end
@implementation NSTimeZone
+ (id)timeZoneForSecondsFromGMT:(NSInteger)s {
    NSTimeZone *z = [[self alloc] init];
    z->_off = s;
    if (s == 0) z->_name = [CFSTR("GMT") retain];
    else {
        char t[24];
        long a = s < 0 ? -s : s;
        snprintf(t, sizeof t, "GMT%c%02ld%02ld", s < 0 ? '-' : '+', a / 3600, a / 60 % 60);
        z->_name = [[NSString stringWithUTF8String:t] retain];
    }
    return [z autorelease];
}
+ (id)timeZoneWithName:(id)n {
    const char *s = [n UTF8String];
    if (!strcmp(s, "UTC") || !strcmp(s, "GMT") || !strcmp(s, "Etc/UTC") || !strcmp(s, "Etc/GMT") || !strcmp(s, "Z")) {
        NSTimeZone *z = [self timeZoneForSecondsFromGMT:0];
        id o = z->_name; z->_name = [n copy]; [o release];
        return z;
    }
    if (!strncmp(s, "GMT", 3) || !strncmp(s, "UTC", 3)) s += 3;
    if (*s == '+' || *s == '-') {
        int sg = *s == '-' ? -1 : 1;
        s++;
        long h = 0, m = 0;
        while (*s >= '0' && *s <= '9' && h < 100) h = h * 10 + (*s++ - '0');
        if (h >= 100) { m = h % 100; h /= 100; }
        if (*s == ':') { s++; while (*s >= '0' && *s <= '9') m = m * 10 + (*s++ - '0'); }
        return [self timeZoneForSecondsFromGMT:sg * (h * 3600 + m * 60)];
    }
    return nil;
}
+ (id)timeZoneWithAbbreviation:(id)a { return [self timeZoneWithName:a]; }
+ (id)localTimeZone { return [self timeZoneWithName:CFSTR("UTC")]; }
+ (id)systemTimeZone { return [self localTimeZone]; }
+ (id)defaultTimeZone { return [self localTimeZone]; }
+ (void)setDefaultTimeZone:(id)z { (void)z; }
+ (void)resetSystemTimeZone {}
+ (id)knownTimeZoneNames { return [NSArray arrayWithObjects:CFSTR("GMT"), CFSTR("UTC"), nil]; }
- (NSInteger)secondsFromGMT { return _off; }
- (NSInteger)secondsFromGMTForDate:(id)d { (void)d; return _off; }
- (double)daylightSavingTimeOffset { return 0; }
- (BOOL)isDaylightSavingTime { return NO; }
- (id)name { return _name; }
- (id)abbreviation { return _off ? _name : CFSTR("GMT"); }
- (id)abbreviationForDate:(id)d { (void)d; return [self abbreviation]; }
- (id)description { return _name; }
- (BOOL)isEqual:(id)o { return cf_is(o, [NSTimeZone class]) && ((NSTimeZone *)o)->_off == _off; }
- (BOOL)isEqualToTimeZone:(id)o { return [self isEqual:o]; }
- (NSUInteger)hash { return (NSUInteger)_off; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (void)dealloc { [_name release]; [super dealloc]; }
@end
