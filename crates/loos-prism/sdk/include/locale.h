#ifndef _LOCALE_H
#define _LOCALE_H
#include <_prism_cdefs.h>
struct lconv {
    char *decimal_point, *thousands_sep, *grouping, *int_curr_symbol, *currency_symbol,
         *mon_decimal_point, *mon_thousands_sep, *mon_grouping, *positive_sign, *negative_sign;
    char int_frac_digits, frac_digits, p_cs_precedes, p_sep_by_space, n_cs_precedes, n_sep_by_space,
         p_sign_posn, n_sign_posn, int_p_cs_precedes, int_n_cs_precedes, int_p_sep_by_space,
         int_n_sep_by_space, int_p_sign_posn, int_n_sign_posn;
};
#define LC_ALL 0
#define LC_COLLATE 1
#define LC_CTYPE 2
#define LC_MONETARY 3
#define LC_NUMERIC 4
#define LC_TIME 5
#define LC_MESSAGES 6
#define LC_COLLATE_MASK (1 << 0)
#define LC_CTYPE_MASK (1 << 1)
#define LC_MESSAGES_MASK (1 << 2)
#define LC_MONETARY_MASK (1 << 3)
#define LC_NUMERIC_MASK (1 << 4)
#define LC_TIME_MASK (1 << 5)
#define LC_ALL_MASK 0x3f
#define LC_GLOBAL_LOCALE ((locale_t)-1)
__BEGIN_DECLS
char *setlocale(int, const char *);
struct lconv *localeconv(void);
__END_DECLS
#include <xlocale.h>
#endif
