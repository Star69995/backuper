import type { FileError } from "../types";

/** A kind of copy failure, explained for the end user: what happened, why, and what to do. */
export interface ErrorKind {
  id: string;
  title: string;
  why: string;
  fix: string;
  /** "warn" = usually harmless (nothing was lost), "bad" = something needs attention. */
  tone: "warn" | "bad";
}

const KINDS: { kind: ErrorKind; codes: number[] }[] = [
  {
    codes: [2, 3],
    kind: {
      id: "gone",
      tone: "warn",
      title: "קבצים שנמחקו או הועברו בזמן הגיבוי",
      why: "הקבצים היו קיימים כשהגיבוי התחיל, אבל נמחקו או הועברו לפני שהגיע תורם להיות מועתקים. זה קורה בדרך כלל בקבצים זמניים שתוכנה יוצרת ומוחקת כל הזמן - קבצי בנייה, מטמון (cache) וקבצים זמניים.",
      fix: "בדרך כלל אין צורך לעשות דבר: הקבצים כבר לא קיימים, כך שאין מה לגבות. אם הם בתיקייה של קבצים זמניים, אפשר להחריג אותה במסננים של המשימה כדי שהשגיאה לא תחזור.",
    },
  },
  {
    codes: [32, 33],
    kind: {
      id: "inUse",
      tone: "bad",
      title: "קבצים שפתוחים בתוכנה אחרת",
      why: "תוכנה אחרת (למשל Outlook, מסד נתונים, מכונה וירטואלית או תוכנה שעדיין רצה) מחזיקה את הקבצים פתוחים ולא מאפשרת לקרוא אותם.",
      fix: "סגרו את התוכנה שמשתמשת בקבצים והריצו את הגיבוי שוב, או תזמנו את הגיבוי לשעה שבה התוכנה בדרך כלל סגורה.",
    },
  },
  {
    codes: [5],
    kind: {
      id: "access",
      tone: "bad",
      title: "אין הרשאת גישה",
      why: "Windows לא מרשה לקרוא את הקבצים או לכתוב אותם ליעד - למשל קבצי מערכת, תיקיות של משתמש אחר, או קובץ ביעד שמסומן לקריאה בלבד.",
      fix: "אם הקבצים לא חשובים לגיבוי, החריגו אותם במסננים של המשימה. אחרת, בדקו את ההרשאות של התיקייה (לחיצה ימנית > מאפיינים > אבטחה).",
    },
  },
  {
    codes: [39, 112],
    kind: {
      id: "diskFull",
      tone: "bad",
      title: "אין מספיק מקום בכונן היעד",
      why: "כונן היעד התמלא באמצע הגיבוי.",
      fix: "פנו מקום בכונן היעד, הקטינו את מספר הגיבויים שנשמרים בהגדרות המשימה, או בחרו כונן יעד גדול יותר. אחר כך הריצו את הגיבוי שוב.",
    },
  },
  {
    codes: [21, 53, 55, 59, 64, 67, 1167, 1231],
    kind: {
      id: "disconnected",
      tone: "bad",
      title: "הכונן או מיקום הרשת לא זמינים",
      why: "הכונן נותק, נכנס למצב שינה, או שהחיבור לרשת נפל באמצע הגיבוי.",
      fix: "ודאו שהכונן מחובר ושהחיבור יציב (כבל, מפצל USB, רשת), והריצו את הגיבוי שוב.",
    },
  },
  {
    codes: [23, 483, 1117, 1392],
    kind: {
      id: "readError",
      tone: "bad",
      title: "שגיאת קריאה מהדיסק",
      why: "הדיסק לא הצליח לקרוא את הקבצים. ייתכן שיש בו אזורים פגומים, או שהקבצים עצמם פגומים.",
      fix: "זה סימן אזהרה לגבי תקינות הדיסק. הריצו בדיקת דיסק (לחיצה ימנית על הכונן > מאפיינים > כלים > בדיקה). אם השגיאה חוזרת, כדאי להעתיק את המידע ולהחליף את הדיסק בהקדם.",
    },
  },
  {
    codes: [206],
    kind: {
      id: "longPath",
      tone: "bad",
      title: "נתיב ארוך מדי",
      why: "הנתיב המלא של הקבצים (תיקיות + שם הקובץ) ארוך יותר ממה ש-Windows מאפשר.",
      fix: "קצרו את שמות התיקיות או הקבצים, או העבירו אותם למיקום עם נתיב קצר יותר.",
    },
  },
  {
    codes: [123],
    kind: {
      id: "badName",
      tone: "bad",
      title: "שם קובץ לא חוקי",
      why: "שם הקובץ מכיל תווים ש-Windows לא מאפשר. זה קורה בדרך כלל בקבצים שנוצרו במחשב או במערכת אחרת.",
      fix: "שנו את שם הקובץ כך שלא יכיל תווים כמו \\ / : * ? \" < > |.",
    },
  },
  {
    codes: [225, 226],
    kind: {
      id: "virus",
      tone: "bad",
      title: "נחסם על ידי האנטי-וירוס",
      why: "תוכנת האנטי-וירוס זיהתה את הקבצים כחשודים וחסמה את הגישה אליהם.",
      fix: "סרקו את הקבצים באנטי-וירוס. אם הם תקינים, הוסיפו להם חריגה באנטי-וירוס. אם לא - עדיף שלא יהיו בגיבוי.",
    },
  },
  {
    codes: [362, 363, 364, 365, 366, 380, 381, 382, 383, 384, 385, 386, 387, 388, 389, 390, 391, 392, 393, 394, 395, 396, 1920],
    kind: {
      id: "cloud",
      tone: "bad",
      title: "קבצי ענן שלא זמינים במחשב",
      why: "הקבצים שמורים בענן (OneDrive וכדומה) ולא הורדו למחשב, או ששירות הסנכרון לא פועל.",
      fix: "ודאו ש-OneDrive (או שירות הסנכרון) פועל, ובתיקייה הזאת בחרו 'השאר תמיד במכשיר זה', כדי שהקבצים יהיו זמינים לגיבוי.",
    },
  },
];

const OTHER: ErrorKind = {
  id: "other",
  tone: "bad",
  title: "שגיאות אחרות",
  why: "Windows דיווח על שגיאה בהעתקת הפריטים האלה. ההודעה המקורית מופיעה ליד כל אחד מהם.",
  fix: "הריצו את הגיבוי שוב. אם השגיאה חוזרת, פתחו את היומן המפורט לפרטים נוספים.",
};

/** Every kind, in display order (for the help page). */
export const ERROR_KINDS: ErrorKind[] = [...KINDS.map((k) => k.kind), OTHER];

const BY_CODE =new Map(KINDS.flatMap(({ kind, codes }) => codes.map((c) => [c, kind] as const)));

export function errorKind(e: FileError): ErrorKind {
  return (e.code != null && BY_CODE.get(e.code)) || OTHER;
}

/** Errors grouped by kind; problems that need attention first, harmless ones last. */
export function groupFileErrors(errors: FileError[]): { kind: ErrorKind; errors: FileError[] }[] {
  const groups = new Map<string, { kind: ErrorKind; errors: FileError[] }>();
  for (const e of errors) {
    const kind = errorKind(e);
    const g = groups.get(kind.id) ?? groups.set(kind.id, { kind, errors: [] }).get(kind.id)!;
    g.errors.push(e);
  }
  return [...groups.values()].sort((a, b) => (a.kind.tone === b.kind.tone ? 0 : a.kind.tone === "bad" ? -1 : 1));
}
