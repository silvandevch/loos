#include "compat.h"
#include <stdio.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { long long a_ = (long long)(a), b_ = (long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %lld, want %lld\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

static void test_versions(void)
{
    compat_era e;
    const compat_version *v;
    CHECK_EQ(compat_version_of(COMPAT_ERA_WIN7)->major, 6);   CHECK_EQ(compat_version_of(COMPAT_ERA_WIN7)->minor, 1);
    CHECK_EQ(compat_version_of(COMPAT_ERA_WIN7)->build, 7601); CHECK_EQ(compat_version_of(COMPAT_ERA_WIN7)->sp_major, 1);
    CHECK_EQ(compat_version_of(COMPAT_ERA_WIN8)->build, 9200); CHECK_EQ(compat_version_of(COMPAT_ERA_WIN81)->minor, 3);
    v = compat_version_of(COMPAT_ERA_WIN10); CHECK_EQ(v->major, 10); CHECK_EQ(v->minor, 0); CHECK_EQ(v->build, 19045);
    v = compat_version_of(COMPAT_ERA_WIN11); CHECK_EQ(v->major, 10); CHECK(v->build > 22000);       /* 11 is 10.0.2xxxx */
    CHECK_EQ(compat_version_of((compat_era)99)->era, COMPAT_NATIVE_ERA);
    CHECK(compat_era_parse("Windows 7", &e) && e == COMPAT_ERA_WIN7);
    CHECK(compat_era_parse("WIN8.1", &e) && e == COMPAT_ERA_WIN81);
    CHECK(compat_era_parse("8", &e) && e == COMPAT_ERA_WIN8);
    CHECK(compat_era_parse("win-11", &e) && e == COMPAT_ERA_WIN11);
    CHECK(compat_era_parse("10", &e) && e == COMPAT_ERA_WIN10);
    CHECK(!compat_era_parse("vista", &e)); CHECK(!compat_era_parse("", &e)); CHECK(!compat_era_parse(NULL, &e));

    CHECK(compat_shim_has(COMPAT_ERA_WIN7, COMPAT_SHIM_VERSION_REPORT));
    CHECK(compat_shim_has(COMPAT_ERA_WIN7, COMPAT_SHIM_DPI_AWARE));
    CHECK(!compat_shim_has(COMPAT_ERA_WIN7, COMPAT_SHIM_PER_MONITOR_DPI_V1));
    CHECK(compat_shim_has(COMPAT_ERA_WIN81, COMPAT_SHIM_PER_MONITOR_DPI_V1));
    CHECK(!compat_shim_has(COMPAT_ERA_WIN81, COMPAT_SHIM_PER_MONITOR_DPI_V2));
    CHECK(compat_shim_has(COMPAT_ERA_WIN10, COMPAT_SHIM_PER_MONITOR_DPI_V2));
    CHECK(compat_shim_has(COMPAT_ERA_WIN10, COMPAT_SHIM_LONG_PATH_AWARE));
    CHECK(compat_shim_has((compat_era)99, COMPAT_SHIM_LONG_PATH_AWARE));
    CHECK(!compat_shim_has(COMPAT_ERA_WIN11, (enum compat_shim_feature)0));
}

static void test_manifest(void)
{
    compat_era e;
    static const char w10[] = "<supportedOS Id=\"{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}\"/>";
    static const char mixed[] =
        "<?xml version='1.0'?><!-- hi --><assembly><compatibility><application>\n"
        "  <supportedOS   id = '{E2011457-1546-43C5-A5FE-008DEEE3D3F0}' />\n"
        "  <SupportedOS Id=\"{1f676c76-80e1-4239-95bb-83d0f6d0da78}\"></SupportedOS>\n"
        "  <supportedOS Id=\"{35138b9a-5d96-4fbd-8e2d-a2440225f93a}\"/>\n</application></compatibility></assembly>";
    static const char nobraces[] = "<supportedOS Id=\"4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38\"/>";
    static const char unknown[] = "<supportedOS Id=\"{00000000-0000-0000-0000-000000000000}\"/>";
    static const char other_id[] = "<assemblyIdentity id=\"{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}\"/>";
    static const char trunc[] = "<supportedOS Id=\"{8e0f7a12-bfb3-4fe8";

    CHECK(compat_manifest_era(w10, sizeof w10 - 1, &e) && e == COMPAT_NATIVE_ERA);
    CHECK(compat_manifest_era(mixed, sizeof mixed - 1, &e) && e == COMPAT_ERA_WIN81);   /* highest of Vista, 8.1, 7 */
    CHECK(compat_manifest_era(nobraces, sizeof nobraces - 1, &e) && e == COMPAT_ERA_WIN8);
    CHECK(!compat_manifest_era(unknown, sizeof unknown - 1, &e));
    CHECK(!compat_manifest_era(other_id, sizeof other_id - 1, &e));                      /* GUID outside supportedOS */
    CHECK(!compat_manifest_era(trunc, sizeof trunc - 1, &e));
    CHECK(!compat_manifest_era("", 0, &e));
    CHECK(!compat_manifest_era("supportedOS", 11, &e));
}

static void test_db(void)
{
    compat_db db;
    compat_claim c;
    const compat_app *a;
    char buf[1024], out2[1024], to_dll[64], to_sym[64];
    unsigned line = 0;
    static const char w10m[] = "<supportedOS Id=\"{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}\"/>";
    static const char w7m[] = "<supportedOS Id=\"{35138b9a-5d96-4fbd-8e2d-a2440225f93a}\"/>";

    compat_db_init(&db);
    CHECK_EQ(db.default_era, COMPAT_ERA_WIN11);

    /* resolution order: profile > manifest > default */
    compat_resolve(&db, "C:\\Apps\\a.exe", NULL, 0, &c);
    CHECK_EQ(c.era, COMPAT_ERA_WIN11); CHECK_EQ(c.source, COMPAT_SRC_DEFAULT);
    compat_resolve(&db, "C:\\Apps\\a.exe", w7m, sizeof w7m - 1, &c);
    CHECK_EQ(c.era, COMPAT_ERA_WIN7); CHECK_EQ(c.source, COMPAT_SRC_MANIFEST); CHECK(strstr(c.reason, "win7"));
    compat_resolve(&db, "C:\\Apps\\a.exe", w10m, sizeof w10m - 1, &c);
    CHECK_EQ(c.era, COMPAT_ERA_WIN11);                                                    /* 10 GUID = native build */
    CHECK_EQ(compat_db_set_claim(&db, "A.EXE", COMPAT_ERA_WIN81), COMPAT_OK);
    compat_resolve(&db, "C:\\Apps\\a.exe", w7m, sizeof w7m - 1, &c);
    CHECK_EQ(c.era, COMPAT_ERA_WIN81); CHECK_EQ(c.source, COMPAT_SRC_PROFILE);            /* user override beats manifest */
    compat_resolve(&db, "D:/other/A.exe", NULL, 0, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN81);   /* base-name match, '/' tolerated */
    compat_resolve(&db, NULL, w7m, sizeof w7m - 1, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN7);

    /* full-path entry beats base-name entry */
    CHECK_EQ(compat_db_set_claim(&db, "C:\\Games\\A.exe", COMPAT_ERA_WIN7), COMPAT_OK);
    compat_resolve(&db, "c:\\games\\a.EXE", NULL, 0, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN7);
    compat_resolve(&db, "c:\\other\\a.exe", NULL, 0, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN81);
    a = compat_db_get_app(&db, "A.exe"); CHECK(a && a->era == COMPAT_ERA_WIN81);
    CHECK(compat_db_get_app(&db, "zzz.exe") == NULL);
    compat_db_clear_claim(&db, "C:\\Games\\A.exe");
    compat_resolve(&db, "c:\\games\\a.exe", NULL, 0, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN81);
    CHECK_EQ(compat_db_set_claim(&db, "", COMPAT_ERA_WIN7), COMPAT_ERR_NAME);
    CHECK_EQ(compat_db_set_claim(&db, "x.exe", (compat_era)42), COMPAT_ERR_ERA);
    CHECK_EQ(db.napps, 1);

    /* capacity */
    { unsigned i; char n[32]; compat_db big; compat_db_init(&big);
      for (i = 0; i < COMPAT_MAX_APPS; i++) { snprintf(n, sizeof n, "app%u.exe", i); CHECK_EQ(compat_db_set_claim(&big, n, COMPAT_ERA_WIN7), COMPAT_OK); }
      CHECK_EQ(compat_db_set_claim(&big, "one-too-many.exe", COMPAT_ERA_WIN7), COMPAT_ERR_FULL);
      CHECK_EQ(compat_db_set_claim(&big, "APP3.exe", COMPAT_ERA_WIN10), COMPAT_OK);       /* update in place still fine */ }

    /* redirects: app-specific, wildcard, whole DLL, ordinal imports */
    CHECK_EQ(compat_db_add_redirect(&db, "a.exe", "OLD.dll", "Foo", "new.dll", "Bar"), COMPAT_OK);
    CHECK_EQ(compat_db_add_redirect(&db, "*", "legacy.dll", "", "modern.dll", ""), COMPAT_OK);
    CHECK_EQ(compat_db_add_redirect(&db, "a.exe", "x.dll", "S", "y.dll", ""), COMPAT_ERR_SYNTAX);   /* half symbol, half dll */
    CHECK_EQ(compat_db_add_redirect(&db, "a.exe", "dir\\x.dll", "", "y.dll", ""), COMPAT_ERR_NAME);
    CHECK(compat_redirect_import(&db, "C:\\z\\A.exe", "old.dll", "Foo", to_dll, sizeof to_dll, to_sym, sizeof to_sym));
    CHECK(!strcmp(to_dll, "new.dll") && !strcmp(to_sym, "Bar"));
    CHECK(!compat_redirect_import(&db, "b.exe", "old.dll", "Foo", to_dll, sizeof to_dll, to_sym, sizeof to_sym));   /* other app */
    CHECK(!compat_redirect_import(&db, "a.exe", "old.dll", "Other", to_dll, sizeof to_dll, to_sym, sizeof to_sym)); /* other symbol */
    CHECK(!compat_redirect_import(&db, "a.exe", "old.dll", NULL, to_dll, sizeof to_dll, to_sym, sizeof to_sym));    /* ordinal vs symbol rule */
    CHECK(compat_redirect_import(&db, "b.exe", "Legacy.DLL", "Anything", to_dll, sizeof to_dll, to_sym, sizeof to_sym));
    CHECK(!strcmp(to_dll, "modern.dll") && !strcmp(to_sym, "Anything"));                    /* symbol carried over */
    CHECK(compat_redirect_import(&db, "b.exe", "legacy.dll", NULL, to_dll, sizeof to_dll, to_sym, sizeof to_sym));
    CHECK(to_sym[0] == 0);                                                                   /* ordinal: caller keeps it */
    CHECK(!compat_redirect_import(&db, "b.exe", "legacy.dll", "S", to_dll, 4, to_sym, sizeof to_sym));               /* buffer too small: no rule */

    /* text round trip */
    CHECK_EQ(compat_db_set_claim(&db, "C:\\Program Files\\Old App\\run.exe", COMPAT_ERA_WIN7), COMPAT_OK);
    { size_t need = compat_db_dump(&db, buf, sizeof buf); CHECK(need > 0 && need < sizeof buf); }
    { compat_db rt; size_t need; compat_db_init(&rt);
      CHECK_EQ(compat_db_parse(&rt, buf, strlen(buf), &line), COMPAT_OK);
      need = compat_db_dump(&rt, out2, sizeof out2);
      CHECK(need == strlen(buf) && !strcmp(buf, out2));
      compat_resolve(&rt, "c:\\program files\\old app\\run.exe", NULL, 0, &c); CHECK_EQ(c.era, COMPAT_ERA_WIN7); }
    CHECK_EQ(compat_db_dump(&db, buf, 10), compat_db_dump(&db, NULL, 0));                   /* size probe, truncation safe */

    /* parse errors report the line and leave the db untouched */
    { compat_db p; static const char good[] = "default win10\n# comment\napp x.exe 7\n\nredirect * a.dll!f b.dll!g\n";
      compat_db_init(&p);
      CHECK_EQ(compat_db_parse(&p, good, sizeof good - 1, &line), COMPAT_OK);
      CHECK_EQ(p.default_era, COMPAT_ERA_WIN10); CHECK_EQ(p.napps, 1); CHECK_EQ(p.nredirects, 1);
      { static const char bad[] = "app y.exe win7\napp z.exe notanera\n"; unsigned before = p.napps;
        CHECK_EQ(compat_db_parse(&p, bad, sizeof bad - 1, &line), COMPAT_ERR_ERA); CHECK_EQ(line, 2); CHECK_EQ(p.napps, before); }
      { static const char bad[] = "default win7\nfrobnicate\n"; CHECK_EQ(compat_db_parse(&p, bad, sizeof bad - 1, &line), COMPAT_ERR_SYNTAX); CHECK_EQ(line, 2); CHECK_EQ(p.default_era, COMPAT_ERA_WIN10); }
      { static const char bad[] = "app \"unterminated 7\n"; CHECK_EQ(compat_db_parse(&p, bad, sizeof bad - 1, &line), COMPAT_ERR_SYNTAX); CHECK_EQ(line, 1); }
      { static const char bad[] = "redirect * a.dll!f b.dll\n"; CHECK_EQ(compat_db_parse(&p, bad, sizeof bad - 1, &line), COMPAT_ERR_SYNTAX); }
      { static const char crlf[] = "default win7\r\napp q.exe 8.1\r\n"; compat_db c2; compat_db_init(&c2);
        CHECK_EQ(compat_db_parse(&c2, crlf, sizeof crlf - 1, &line), COMPAT_OK); CHECK_EQ(c2.default_era, COMPAT_ERA_WIN7); CHECK(compat_db_get_app(&c2, "q.exe")->era == COMPAT_ERA_WIN81); }
      { char longl[700]; memset(longl, 'a', sizeof longl); CHECK_EQ(compat_db_parse(&p, longl, sizeof longl, &line), COMPAT_ERR_SYNTAX); } }
}

int main(void)
{
    test_versions();
    test_manifest();
    test_db();
    if (fails) printf("compat: %d FAILED\n", fails); else printf("compat: all passed\n");
    return fails != 0;
}
