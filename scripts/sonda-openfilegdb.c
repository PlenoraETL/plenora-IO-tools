/* La capability di OpenFileGDB, provata eseguendo.
 *
 * Il nome del pacchetto non e' una prova: `libgdal-core` potrebbe essere
 * compilato senza il driver, e la versione uguale a quella di Windows non dice
 * nulla sulle opzioni di compilazione. Qui si registra, si crea un FileGDB
 * vero, ci si scrive un layer con un campo e un punto, si chiude, si riapre
 * **da zero**, e si rilegge.
 *
 * I prototipi sono dichiarati qui invece di includere `gdal.h`: il pacchetto
 * runtime non porta gli header, e l'API C di GDAL e' stabile. Dichiararli
 * significa anche che questa sonda prova cio' che la libreria **esporta**, non
 * cio' che un header promette.
 */

#include <stdio.h>
#include <string.h>

typedef void *GDALDatasetH;
typedef void *GDALDriverH;
typedef void *OGRLayerH;
typedef void *OGRFeatureH;
typedef void *OGRFeatureDefnH;
typedef void *OGRFieldDefnH;
typedef void *OGRGeometryH;
typedef void *OGRSpatialReferenceH;

void GDALAllRegister(void);
GDALDriverH GDALGetDriverByName(const char *);
GDALDatasetH GDALCreate(GDALDriverH, const char *, int, int, int, int, char **);
GDALDatasetH GDALOpenEx(const char *, unsigned int, const char *const *,
                        const char *const *, const char *const *);
void GDALClose(GDALDatasetH);
OGRLayerH GDALDatasetCreateLayer(GDALDatasetH, const char *, OGRSpatialReferenceH, int, char **);
OGRLayerH GDALDatasetGetLayer(GDALDatasetH, int);
int GDALDatasetGetLayerCount(GDALDatasetH);
OGRFieldDefnH OGR_Fld_Create(const char *, int);
void OGR_Fld_Destroy(OGRFieldDefnH);
int OGR_L_CreateField(OGRLayerH, OGRFieldDefnH, int);
OGRFeatureDefnH OGR_L_GetLayerDefn(OGRLayerH);
OGRFeatureH OGR_F_Create(OGRFeatureDefnH);
void OGR_F_Destroy(OGRFeatureH);
void OGR_F_SetFieldString(OGRFeatureH, int, const char *);
const char *OGR_F_GetFieldAsString(OGRFeatureH, int);
int OGR_F_SetGeometry(OGRFeatureH, OGRGeometryH);
OGRGeometryH OGR_F_GetGeometryRef(OGRFeatureH);
int OGR_L_CreateFeature(OGRLayerH, OGRFeatureH);
void OGR_L_ResetReading(OGRLayerH);
OGRFeatureH OGR_L_GetNextFeature(OGRLayerH);
int OGR_FD_GetFieldCount(OGRFeatureDefnH);
OGRGeometryH OGR_G_CreateGeometry(int);
void OGR_G_DestroyGeometry(OGRGeometryH);
void OGR_G_SetPoint_2D(OGRGeometryH, int, double, double);
double OGR_G_GetX(OGRGeometryH, int);
double OGR_G_GetY(OGRGeometryH, int);
int OGR_G_GetGeometryType(OGRGeometryH);
const char *OGR_Dr_GetName(GDALDriverH);

#define OFTString 4
#define wkbPoint 1
#define GDAL_OF_VECTOR 0x04
#define GDAL_OF_READONLY 0x00

static int fallito(const char *messaggio) {
    fprintf(stderr, "ROSSO: %s\n", messaggio);
    return 1;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        return fallito("uso: sonda <percorso.gdb>");
    }
    const char *percorso = argv[1];

    GDALAllRegister();

    /* 1. il driver esiste. */
    GDALDriverH driver = GDALGetDriverByName("OpenFileGDB");
    if (driver == NULL) {
        return fallito("OpenFileGDB non e' registrato: la libreria non lo porta");
    }
    printf("driver registrato: %s\n", OGR_Dr_GetName(driver));

    /* 2. crea davvero. */
    GDALDatasetH ds = GDALCreate(driver, percorso, 0, 0, 0, 0, NULL);
    if (ds == NULL) {
        return fallito("OpenFileGDB e' registrato ma non sa creare: capability assente");
    }
    OGRLayerH layer = GDALDatasetCreateLayer(ds, "punti", NULL, wkbPoint, NULL);
    if (layer == NULL) {
        GDALClose(ds);
        return fallito("il driver non ha creato il layer");
    }
    OGRFieldDefnH campo = OGR_Fld_Create("etichetta", OFTString);
    if (OGR_L_CreateField(layer, campo, 1) != 0) {
        OGR_Fld_Destroy(campo);
        GDALClose(ds);
        return fallito("il driver non ha creato il campo");
    }
    OGR_Fld_Destroy(campo);

    OGRFeatureH f = OGR_F_Create(OGR_L_GetLayerDefn(layer));
    OGR_F_SetFieldString(f, 0, "uno");
    OGRGeometryH punto = OGR_G_CreateGeometry(wkbPoint);
    OGR_G_SetPoint_2D(punto, 0, 12.5, 45.9);
    OGR_F_SetGeometry(f, punto);
    if (OGR_L_CreateFeature(layer, f) != 0) {
        return fallito("il driver non ha scritto la riga");
    }
    OGR_G_DestroyGeometry(punto);
    OGR_F_Destroy(f);
    GDALClose(ds);

    /* 3. riapre da zero e rilegge. */
    GDALDatasetH riletto =
        GDALOpenEx(percorso, GDAL_OF_VECTOR | GDAL_OF_READONLY, NULL, NULL, NULL);
    if (riletto == NULL) {
        return fallito("il FileGDB creato non si riapre");
    }
    if (GDALDatasetGetLayerCount(riletto) != 1) {
        GDALClose(riletto);
        return fallito("il numero di layer riletti non torna");
    }
    OGRLayerH riletto_layer = GDALDatasetGetLayer(riletto, 0);
    OGRFeatureDefnH defn = OGR_L_GetLayerDefn(riletto_layer);
    if (OGR_FD_GetFieldCount(defn) < 1) {
        GDALClose(riletto);
        return fallito("lo schema riletto non porta il campo");
    }

    OGR_L_ResetReading(riletto_layer);
    int righe = 0;
    int geometria_giusta = 0;
    int etichetta_giusta = 0;
    OGRFeatureH letta;
    while ((letta = OGR_L_GetNextFeature(riletto_layer)) != NULL) {
        righe++;
        const char *etichetta = OGR_F_GetFieldAsString(letta, 0);
        if (etichetta != NULL && strcmp(etichetta, "uno") == 0) {
            etichetta_giusta = 1;
        }
        OGRGeometryH g = OGR_F_GetGeometryRef(letta);
        if (g != NULL && OGR_G_GetGeometryType(g) == wkbPoint) {
            double x = OGR_G_GetX(g, 0), y = OGR_G_GetY(g, 0);
            if (x > 12.4 && x < 12.6 && y > 45.8 && y < 46.0) {
                geometria_giusta = 1;
            }
        }
        OGR_F_Destroy(letta);
    }
    GDALClose(riletto);

    if (righe != 1) {
        return fallito("il numero di righe rilette non torna");
    }
    if (!etichetta_giusta) {
        return fallito("l'attributo riletto non torna");
    }
    if (!geometria_giusta) {
        return fallito("la geometria riletta non torna");
    }

    printf("VERDE: creato, riletto; 1 layer, 1 campo, 1 riga, geometria a posto\n");
    return 0;
}
