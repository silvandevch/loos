#ifndef _LOCALE_H
#define _LOCALE_H
#include "features.h"
#include <stddef.h>
__LOOS_BEGIN
#define LC_CTYPE 0
#define LC_NUMERIC 1
#define LC_TIME 2
#define LC_COLLATE 3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL 6
struct lconv {
    char *decimal_point, *thousands_sep, *grouping, *int_curr_symbol, *currency_symbol,
         *mon_decimal_point, *mon_thousands_sep, *mon_grouping, *positive_sign, *negative_sign;
    char int_frac_digits, frac_digits, p_cs_precedes, p_sep_by_space, n_cs_precedes, n_sep_by_space,
         p_sign_posn, n_sign_posn, int_p_cs_precedes, int_p_sep_by_space, int_n_cs_precedes,
         int_n_sep_by_space, int_p_sign_posn, int_n_sign_posn;
};
char *setlocale(int, const char *);
struct lconv *localeconv(void);
__LOOS_END
#endif
