#include "shell.h"
#include <string.h>

void inventory_init(shell_inventory *inv) { memset(inv, 0, sizeof *inv); }

bool inventory_add(shell_inventory *inv, const char *name, const char *target, const char *keywords, shell_item_kind kind)
{
    if (inv->n >= SHELL_MAX_APPS || !name || !*name) return false;
    shell_item *it = &inv->item[inv->n];
    memset(it, 0, sizeof *it);
    strncpy(it->name, name, sizeof it->name - 1);
    strncpy(it->target, target ? target : "", sizeof it->target - 1);
    strncpy(it->keywords, keywords ? keywords : "", sizeof it->keywords - 1);
    it->kind = kind;
    inv->n++;
    return true;
}

static char *trim(char *s)
{
    while (*s == ' ' || *s == '\t') s++;
    char *e = s + strlen(s);
    while (e > s && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r')) *--e = 0;
    return s;
}

int inventory_parse(shell_inventory *inv, const char *text)
{
    int added = 0;
    while (*text) {
        char line[320];
        size_t n = strcspn(text, "\n");
        if (n >= sizeof line) n = sizeof line - 1;
        memcpy(line, text, n); line[n] = 0;
        text += strcspn(text, "\n"); if (*text) text++;
        char *p = trim(line);
        if (!*p || *p == '#') continue;
        char *f[4] = { 0 }; int k = 0;
        for (char *tok = p; k < 4;) {
            f[k++] = tok;
            char *bar = strchr(tok, '|');
            if (!bar) break;
            *bar = 0; tok = bar + 1;
        }
        if (k < 3) continue;                                    /* kind|name|target[|keywords] */
        shell_item_kind kind;
        char *kd = trim(f[0]);
        if (!strcmp(kd, "app")) kind = ITEM_APP; else if (!strcmp(kd, "file")) kind = ITEM_FILE; else if (!strcmp(kd, "setting")) kind = ITEM_SETTING; else continue;
        if (inventory_add(inv, trim(f[1]), trim(f[2]), k > 3 ? trim(f[3]) : "", kind)) added++;
    }
    return added;
}

void inventory_defaults(shell_inventory *inv)
{
    static const char defaults[] =
        "# built-in LoOS apps and settings pages (no web results, ever)\n"
        "app|LoOS Calculator|calc.exe|math arithmetic scientific\n"
        "app|LoCmd|loos-cmd.exe|terminal shell console prompt cmd\n"
        "app|LoOS Notepad|loos-notepad.exe|text editor notes plugins\n"
        "app|LoOS Task Manager|loos-taskmgr.exe|processes monitor cpu memory kill\n"
        "setting|Privacy|settings:privacy|telemetry permissions camera microphone location\n"
        "setting|Network firewall|settings:firewall|connections block allow sockets\n"
        "setting|App permissions|settings:permissions|files notifications camera mic\n"
        "setting|Updates|settings:updates|mirror offline opt-in\n"
        "setting|Appearance|settings:appearance|theme dark light accent scale dpi wallpaper\n"
        "setting|Compatibility|settings:compat|windows version 7 10 11 shim\n"
        "setting|Accounts|settings:accounts|users sign-in password\n"
        "setting|Accessibility|settings:accessibility|contrast narrator keyboard magnifier motion\n"
        "setting|Display|settings:display|resolution monitors scale dpi\n";
    inventory_parse(inv, defaults);
}
