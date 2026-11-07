title = Paketinstallation
reading = Läser paketet…
invalid = Filen är inte ett giltigt Debian-paket.
invalid-hint = Hämtningen kan vara ofullständig. Försök hämta den igen.
wrong-architecture = ”{ $name }” är byggt för { $arch }. Den här datorn behöver ett paket för { $native }.
wrong-architecture-hint = Leta efter en hämtning för { $native } på utvecklarens webbplats.
arch-amd64 = Intel- och AMD-datorer (amd64)
arch-arm64 = 64-bitars ARM (arm64)
arch-armhf = 32-bitars ARM (armhf)
arch-i386 = äldre 32-bitars Intel- och AMD-datorer (i386)
version-size = Version { $version } · { $size }
package-facts = { $package } · { $facts }
present = Version { $version } är installerad.
upgrade-from = Uppgraderar från { $from } till { $to }
downgrade-from = Nedgraderar från { $from } till { $to }
downgrade-caution = Den här versionen är äldre än den installerade versionen { $from }.
also-installs = { $count ->
    [one] Installerar även 1 paket
   *[other] Installerar även { $count } paket
}
also-installs-download = { $count ->
    [one] Installerar även 1 paket · { $size } att hämta
   *[other] Installerar även { $count } paket · { $size } att hämta
}
removes = { $count ->
    [one] Tar bort 1 paket: { $names }
   *[other] Tar bort { $count } paket: { $names }
}
trust = Det här paketet kommer inte från dina programvarukällor. Dess installationsskript får full kontroll över datorn, och paketet kan lägga till en egen programvarukälla. Installera det bara om du litar på var du fick det ifrån.
signed-available = Samma version finns i dina programvarukällor ({ $source }), där den är signerad. Det är säkrare att installera den därifrån än från den här filen.
origin-debian = Debian { $suite }, { $component }
install-file = Installera den här filen
install-signed = Installera från programvarukällor
from-maintainer = Från { $maintainer }
blocked = Det går inte att installera: paketet behöver andra paket som inte är tillgängliga
needs = Behöver { $names }
details = Detaljer
packages = Paket
status-waiting = Väntar på att en annan installation ska bli klar…
status-authorizing = Väntar på ditt lösenord…
authorize-hint = Ange en administratörs lösenord i dialogrutan för att fortsätta, eller avbryt den för att stoppa.
password-hint = Installationen kräver en administratörs lösenord.
status-downloading = Hämtar…
status-installing = Installerar…
status-removing = Tar bort…
status-working = Arbetar…
install = Installera
install-and-remove = Installera och ta bort
cancel = Avbryt
close = Stäng
done = Klar
installed = ”{ $name }” är installerat.
restart-to-finish = Starta om datorn för att slutföra installationen.
open-app = Öppna { $app }
remove = Ta bort…
remove-confirm = Vill du ta bort ”{ $name }”?
remove-also = { $count ->
    [one] Detta tar också bort 1 paket: { $names }
   *[other] Detta tar också bort { $count } paket: { $names }
}
remove-button = Ta bort
removed = ”{ $name }” har tagits bort.
failed-locked = En annan installation pågår. Försök igen när den är klar.
failed-system = Det här paketet är en del av systemet och kan inte tas bort.
failed-unavailable = Installation av paket kräver PackageKit, som inte är installerat.
failed-no-network = Det gick inte att hämta de paket som behövs. Kontrollera din internetanslutning och försök igen.
failed-no-space = Det finns inte tillräckligt med diskutrymme. Frigör lite utrymme och försök igen.
failed-dependencies = Paketet behöver andra paket som inte kan installeras just nu. Installera väntande uppdateringar i Inställningar › Programuppdatering och försök sedan igen.
failed-install = Det gick inte att installera paketet. Försök igen, eller öppna Detaljer för att se vad som gick fel.
failed-remove = Det gick inte att ta bort paketet. Försök igen, eller öppna Detaljer för att se vad som gick fel.
try-again = Försök igen
not-authorized = Ingenting har ändrats eftersom inget administratörslösenord angavs. Om inget lösenordsfönster visades, logga ut och in igen och försök sedan på nytt.
