# EvoParseq QoS — külön hálózati mérőprogram

Ez szintetikus TCP/TLS-mérő, nem Parsec-kliens. Az EvoParseq alkalmazásba nem
fordul bele, nem olvas bejelentkezést vagy settings.json-t, és nem módosítja a
Parsec hostot. Telepítés nélkül, normál felhasználóként futtatható.

## Használat

1. Csomagold ki a ZIP-et mindkét gépen egy írható könyvtárba.
2. A másik gépen indítsd az **EvoParseq-QoS-Test.exe** fájlt, és válaszd az
   **1 — receiver** módot. Hagyd nyitva az ablakát.
3. Az ott létrejött **qos-peer.json** fájlt másold a kliensgépen lévő teszt-EXE
   mellé. Új receiver-indításkor új párosítás készül; ilyenkor újra másold át.
4. A kliensen indítsd az EXE-t, válaszd a **2 — client** módot, és add meg a
   receiver gép **LAN/VPN IPv4-címét**. A két gép között a TCP **49192 és 49193**
   portnak elérhetőnek kell lennie. A program nem hoz létre tűzfalszabályt.
5. A mérés idejére állítsd le a streamet és az egyéb sebességteszteket. A program
   maga hozza létre a feltöltési/letöltési terhelést; körülbelül **7 percig** fut.
6. A végén a kliens **qos-results-....json** fájlját küldd el elemzésre.
   A párosító qos-peer.json megosztására ehhez nincs szükség.

A receiver a konzol bezárásával / Ctrl+C-vel leállítható. Egyedi porthoz:
`EvoParseq-QoS-Test.exe server 49192`. Parancssori kliens:
`EvoParseq-QoS-Test.exe client 192.168.1.50 10` (5–30 másodperc/próba).

## Mit mér?

- TCP és tanúsítvány-ellenőrzött TLS külön; rövid, szintetikus üzenetek RTT-je.
- Terheletlen, feltöltéssel terhelt, letöltéssel terhelt és kétirányban terhelt út.
- Minden esetben **ki / be / be / ki** QoS-sorrend, új kapcsolatokon.
- p50/p95/p99, maximum, mintaszám, időtúllépések és az egyedi RTT-minták.
- A Windows által visszaadott QoS-státusz, konfigurált DSCP/L2 érték és hibakód.

Csak a kliens **mérőkapcsolata** kap AudioVideo, non-adaptive qWAVE-jelölést.
A terhelési és receiver-oldali kapcsolatok jelöletlenek. Nincs sávszélesség-
szabályozás vagy qWAVE-útvonalpróba. Minden terhelési kapcsolatnak fel kell állnia
a bemelegítés és mintavétel előtt; a TLS/párosítás ideje nem kerül az RTT-be.
A Windows-terhelési socketek próba végi abortív zárása a következő A/B próbába
átcsorgó, pufferelt terhelést csökkenti; a mért echo-kapcsolatra nem vonatkozik.

Két bulk kapcsolat fut irányonként. A feltöltési bájtszám a socketbe beadott
adatot, a letöltési a fogadott adatot számolja; ezek nem egyenértékű áteresztési
mérőszámok. Hibás / fel nem állt terhelésű próbából nem vonható le QoS-előny.

## Az eredmény határai

- A sikeres QoS API és a konfigurált DSCP nem igazolja a hálózaton ténylegesen
  megjelenő jelölést vagy a router prioritási sorát. A VPN is módosíthatja /
  eldobhatja a jelölést.
- Ha a Windows nem igazolja a jelölést, a bekapcsolt próba kimarad és ezt a JSON
  rögzíti. Ezt nem szabad „nincs javulás” eredménynek tekinteni.
- Loopbacken csak a protokoll/TLS és hibakezelés ellenőrizhető. A Windows ezen
  gyakran elutasítja a QoS-folyamot; ott nem mérünk prioritási előnyt.
- Legalább két teljes sorozat célszerű ugyanazon útvonalon. A kevés mintából
  származó p99 bizonytalan; 100 alatti mintaszámnál különösen óvatosan értékelendő.
- A terhelés CPU- vagy VPN-korlátba is ütközhet. A CPU-terhelést és a tényleges
  hálózati kihasználtságot is érdemes figyelni a két gépen.
- A bulk terhelés külön socketeken fut. Ez nem modellezi az egy lowlat TLS-streamen
  belüli videó/vezérlés multiplexálás sorbanállását; a hálózati QoS lehetőségét
  méri, nem a teljes lowlat implementációt.
- Ez hálózati válaszkészség, **nem** capture-to-display vagy Parsec end-to-end
  késleltetés. A LAN/VPN-eredmény a TURN TCP/TLS-útvonalra nem általánosítható.

A TLS a receiver frissen generált tanúsítványát ellenőrzi a párosító fájl alapján;
nincs tanúsítvány-ellenőrzés kikapcsolása, és nem használunk éles hitelesítő adatot.
A receiver legfeljebb nyolc aktív kapcsolatot, rögzített méretű üzeneteket és
korlátozott idejű terhelési kéréseket fogad. Az alkalmazás működését a próba nem
érinti, az éles QoS-beépítésről csak a mérések után döntünk.

## Fejlesztői ellenőrzés

`cargo test --locked --manifest-path tools/qos-latency/Cargo.toml`

A CI külön ellenőrzi a TCP/TLS echo és bulk adatcserét, a hibás párosítás és
tanúsítvány elutasítását, valamint a percentiliseket. Ezek funkcionális tesztek;
egyikük sem állít hálózati gyorsulást. Az alkalmazás Cargo-függőségei változatlanok.
