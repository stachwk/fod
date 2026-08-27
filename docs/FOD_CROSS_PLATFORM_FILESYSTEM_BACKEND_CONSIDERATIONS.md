# FOD - rozwazania o backendach filesystemu i przenosnosci miedzy systemami

Status: rozwazania architektoniczne. Dokument nie jest planem prac, roadmapa ani deklaracja przyszlej implementacji.

## Cel dokumentu

Dokument zbiera rozwazania wynikajace z testow SELinux/FUSE na Rocky Linux 10.2 oraz z pytania, czy FOD powinien pozostac filesystemem userspace opartym o FUSE, czy tez rozwazyc natywny modul kernela. Uwzglednia tez mozliwosc uruchamiania FOD na Windows i macOS.

Nie przesadza on kierunku rozwoju. Jego celem jest zachowanie argumentow technicznych, ograniczen i potencjalnych wariantow architektury.

## Punkt wyjscia: FOD jako filesystem userspace

Obecny FOD korzysta z modelu userspace filesystemu. Logika filesystemu, komunikacja z PostgreSQL, cache, metadata i wiekszosc mechanizmow wykonuje sie poza kernelem.

Schematycznie:

```text
Linux VFS
   |
   v
FUSE
   |
   v
FOD userspace
   |
   v
PostgreSQL / cache / metadata
```

Ten model dobrze pasuje do FOD, poniewaz PostgreSQL i pozostale zaleznosci sa naturalnie obslugiwane w userspace. Awaria procesu FOD nie oznacza bezposrednio awarii calego kernela.

## Co pokazaly testy SELinux na Rocky Linux 10.2

Rocky Linux 10.2 traktuje zwykly filesystem FUSE zgodnie z polityka SELinux jako filesystem `fusefs_t`.

Praktyczny test operacyjny potwierdzil, ze SELinux rzeczywiscie egzekwuje polityke MAC dla dostepu do FOD. Apache dzialajacy w domenie `httpd_t` nie mogl odczytac danych FOD przy `httpd_use_fusefs=off`, a po wlaczeniu `httpd_use_fusefs=on` otrzymywal dostep do tego samego pliku.

Jednoczesnie zwykly FUSE na testowanym Rocky Linux 10.2 nie zapewnil modelu per-inode `security.selinux` odpowiadajacego XFS/ext4. Proba relabelu byla odrzucana przez warstwe kernel/SELinux przed dotarciem `FUSE_SETXATTR` do callbacku FOD.

To rozroznienie jest wazne:

```text
FUSE/FOD na Rocky 10.2:
  rzeczywiste egzekwowanie SELinux MAC       - tak
  klasyfikacja filesystemu jako fusefs_t     - tak
  per-file security.selinux jak XFS/ext4     - nie w testowanym stosie
```

Ograniczenie to jest jednym z powodow, dla ktorych pojawila sie mysl o natywnym backendzie kernelowym.

## Rozwazanie: natywny modul kernela Linux

Teoretycznie natywny modul kernela moglby zarejestrowac wlasny typ filesystemu, np. `fod`, zamiast korzystac z typu `fuse`.

Mogloby to stworzyc mozliwosc bardziej natywnej integracji z Linux VFS i SELinux, np. polityki odpowiadajacej:

```text
fs_use_xattr fod ...
```

oraz potencjalnie per-inode `security.selinux` w modelu blizszym XFS/ext4.

### Potencjalne zalety

- wlasny typ filesystemu widoczny dla kernela;
- mozliwosc glebszej integracji z VFS;
- potencjalnie pelniejsza integracja z SELinux per inode;
- brak ograniczen wynikajacych bezposrednio z klasyfikacji zwyklego FUSE jako `fusefs_t`;
- mozliwosc implementowania wybranych mechanizmow blizej kernela.

### Istotne koszty i ryzyka

FOD nie jest prostym lokalnym filesystemem blokowym. Jego logika jest mocno zwiazana z PostgreSQL i mechanizmami userspace.

Nie byloby dobrym rozwiazaniem umieszczanie klienta PostgreSQL i rozbudowanej logiki FOD bezposrednio w kernelu. Bardziej realny wariant wygladalby tak:

```text
Linux VFS
   |
   v
fod.ko
   |
   | IPC
   v
fod-daemon
   |
   v
PostgreSQL
```

W takim wariancie konieczne byloby stworzenie i utrzymywanie wlasnego protokolu kernel-userspace. Funkcjonalnie bylby to mechanizm podobny do problemu, ktory FUSE juz rozwiazuje w sposob standardowy.

Dochodziłyby takze dodatkowe obciazenia:

- zgodnosc z kolejnymi wersjami kernela;
- DKMS lub osobne pakiety kmod;
- Secure Boot i podpisywanie modulow;
- roznice miedzy dystrybucjami Linux;
- znacznie trudniejszy debugging;
- ryzyko, ze blad w kodzie filesystemu doprowadzi do awarii kernela;
- oddzielny lifecycle wydan kodu kernelowego i userspace.

Z tego powodu natywny modul kernela mozna traktowac jako interesujace rozwazanie dla bardzo specyficznych potrzeb integracji Linux, ale nie jako oczywisty zamiennik obecnej architektury FUSE.

## Rozwazanie: wspolny core i rozne frontendy systemowe

Bardziej przenosnym wariantem jest oddzielenie logiki FOD od konkretnego API filesystemu.

Schematycznie:

```text
                    FOD core
        PostgreSQL / cache / metadata / locks
                         |
                 filesystem API FOD
                         |
          +--------------+--------------+
          |              |              |
        Linux          Windows        macOS
          |              |              |
        FUSE           WinFsp      macFUSE/FUSE-T
```

W takim modelu FOD zachowuje wspolna logike danych, a roznice platformowe sa obslugiwane przez cienkie adaptery.

To podejscie ma jedna zasadnicza zalete: przenosnosc nie wymaga przepisywania calego filesystemu dla kazdego kernela.

## Windows

Windows nie posiada linuxowego FUSE jako natywnego interfejsu kernelowego, ale istnieje WinFsp - userspace filesystem framework przeznaczony dla Windows.

WinFsp udostepnia:

- natywne API Windows dla filesystemow userspace;
- warstwe zgodnosci z API FUSE;
- integracje z Windows I/O;
- obsluge Security Descriptors, DACL/SACL, SID, reparse points i innych mechanizmow specyficznych dla Windows.

Dla FOD oznacza to, ze potencjalny port nie musialby byc sterownikiem kernelowym `fod.sys`.

Rozwazany model:

```text
Windows I/O
   |
   v
WinFsp
   |
   v
FOD adapter
   |
   v
FOD core
   |
   v
PostgreSQL
```

Istotne jest, ze model bezpieczenstwa Windows nie jest odpowiednikiem SELinux. Windows korzysta z SID, Access Token, Security Descriptor, DACL i SACL. Dlatego nie nalezaloby mechanicznie przenosic `security.selinux` na Windows.

Ewentualny wspolny model FOD powinien rozdzielac metadata niezalezne od platformy od metadanych bezpieczenstwa specyficznych dla systemu operacyjnego.

## macOS

macOS rowniez ma rozwiazania pozwalajace uruchamiac filesystemy zgodne z idea FUSE.

### macFUSE

macFUSE jest klasycznym rozwiazaniem pozwalajacym implementowac filesystemy userspace na macOS z API zblizonym do FUSE.

Z punktu widzenia FOD daje to potencjalnie mozliwosc portowania istniejacego frontendu przy zachowaniu duzej czesci semantyki POSIX.

### FUSE-T

FUSE-T jest alternatywnym podejsciem nastawionym na zachowanie API FUSE przy wykorzystaniu mechanizmow dostepnych w macOS bez klasycznego wlasnego kexta. Backend moze opierac sie m.in. na lokalnym NFS, SMB lub nowszych mechanizmach systemowych.

Z punktu widzenia architektury FOD interesujace jest to, ze rowniez tutaj mozliwe jest zachowanie userspace core i wymiana jedynie adaptera platformowego.

Schematycznie:

```text
macOS
  |
  +-- macFUSE
  |
  +-- FUSE-T
        |
        v
     FOD core
```

Nie nalezy jednak zakladac identycznej semantyki xattr, ACL i zabezpieczen na Linux, Windows i macOS. Kazda platforma powinna miec jawnie okreslony zakres zgodnosci.

## Rozwazanie o bezpieczenstwie wieloplatformowym

Przenosny FOD nie powinien traktowac wszystkich modeli bezpieczenstwa jako jednego wspolnego zestawu xattr.

Linux moze korzystac z:

```text
UID/GID
mode bits
POSIX ACL
SELinux
security.selinux
```

Windows korzysta z:

```text
SID
Access Token
Security Descriptor
DACL
SACL
```

macOS posiada wlasne rozszerzenia modelu POSIX, ACL, xattr i mechanizmy bezpieczenstwa Apple.

Dlatego sensowna abstrakcja moglaby rozdzielac:

```text
FOD metadata wspolne
    |
    +-- podstawowa identity/ownership
    +-- timestamps
    +-- zwykle xattr
    +-- platform security metadata
           |
           +-- Linux SELinux/POSIX ACL
           +-- Windows Security Descriptor
           +-- macOS ACL/xattr/security metadata
```

Nie oznacza to koniecznosci implementacji takiego modelu. Jest to jedynie konsekwencja techniczna, ktora nalezy uwzgledniac przy ocenie przenosnosci.

## FUSE a modul kernela - porownanie rozwazan

| Cecha | FUSE/userspace | Natywny modul Linux |
| --- | --- | --- |
| PostgreSQL i biblioteki userspace | naturalne | wymaga dodatkowego daemona/IPC |
| Izolacja awarii | proces userspace | blad moze dotknac kernel |
| Przenosnosc | wysoka | niska |
| Windows | mozliwy odpowiednik przez WinFsp | potrzebny osobny sterownik Windows |
| macOS | macFUSE/FUSE-T | potrzebna osobna implementacja systemowa |
| Utrzymanie miedzy kernelami | relatywnie proste | istotny koszt |
| SELinux operational enforcement | tak | tak |
| SELinux per-inode jak XFS/ext4 | ograniczone przez stos FUSE/hosta | potencjalnie mozliwe |
| Secure Boot / podpisywanie modulu | zwykle nie dotyczy FOD userspace | istotny temat |

## Dlaczego FUSE pozostaje interesujace mimo ograniczen SELinux

Brak per-inode `security.selinux` na testowanym Rocky Linux 10.2 nie oznacza, ze FUSE jest zlym mechanizmem dla FOD.

Testy pokazaly, ze:

- SELinux nadal realnie egzekwuje polityke MAC dla `fusefs_t`;
- FOD zachowuje userspace architecture dobrze dopasowana do PostgreSQL;
- ten sam model architektoniczny daje potencjalna droge do Windows przez WinFsp;
- macOS posiada rozwiazania zgodne z modelem FUSE;
- nie trzeba przenosic zlozonej logiki FOD do kernela.

Ograniczenie SELinux jest wiec kompromisem miedzy bardzo gleboka integracja z jednym systemem a przenosnoscia i prostota utrzymania.

## Mozliwy wariant hybrydowy

Teoretycznie nic nie wyklucza istnienia kilku frontendow do tego samego FOD core:

```text
                         FOD core
                            |
          +-----------------+------------------+
          |                 |                  |
     Linux FUSE       Linux native        Windows/macOS
      frontend          frontend             frontend
          |                 |                  |
      fusefs_t        wlasny fs type       WinFsp/FUSE
```

Wariant natywny Linux moglby wtedy istniec jako wyspecjalizowany frontend dla srodowisk wymagajacych integracji niedostepnej przez standardowy FUSE, podczas gdy FUSE pozostawalby rozwiazaniem bardziej uniwersalnym.

Jest to jednak rozwazanie koncepcyjne. Dokument nie zaklada, ze taki frontend powinien powstac.

## Wnioski z rozwazan

Obecny userspace/FUSE model ma istotne zalety dla charakteru FOD: pozwala utrzymac PostgreSQL, cache i rozbudowana logike poza kernelem oraz daje naturalna droge do portow na inne systemy.

Natywny modul Linux moglby potencjalnie dac glebsza integracje z VFS i SELinux, szczegolnie w zakresie per-inode labeling, ale kosztem znacznie wiekszej zlozonosci i utraty przenosnosci.

Windows nie wymaga automatycznie tworzenia wlasnego sterownika kernelowego, poniewaz WinFsp pozwala implementowac filesystem w userspace. macOS posiada macFUSE i FUSE-T, wiec rowniez wpisuje sie w model adaptera userspace.

Najwazniejsze jest rozdzielenie dwoch pytan:

1. jaki poziom natywnej integracji z konkretnym systemem operacyjnym jest potrzebny;
2. jak wiele przenosnosci i wspolnej implementacji FOD chce sie zachowac.

Nie ma tu jednej odpowiedzi wynikajacej wylacznie z obecnego ograniczenia SELinux. Test Rocky pokazuje konkretny kompromis standardowego FUSE, ale sam w sobie nie przesadza, ze FOD powinien przejsc na modul kernela.
