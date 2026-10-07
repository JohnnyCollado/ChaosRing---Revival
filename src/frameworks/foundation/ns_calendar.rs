/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSCalendar` and `NSDateComponents`.
//!
//! Minimal stubs sufficient for guests that use the calendar API to extract
//! date components from a date or to record a save timestamp. We don't
//! actually model multiple calendar systems or time zones — we use the
//! Gregorian calendar via `CFAbsoluteTimeGetGregorianDate`.

use crate::frameworks::core_foundation::time::CFAbsoluteTimeGetGregorianDate;
use crate::frameworks::foundation::{NSInteger, NSTimeInterval, NSUInteger};
use crate::objc::{autorelease, id, msg, nil, objc_classes, ClassExports, HostObject, NSZonePtr};

// NSCalendarUnit constants.
#[allow(dead_code)]
pub const NSEraCalendarUnit: NSUInteger = 1 << 1;
#[allow(dead_code)]
pub const NSYearCalendarUnit: NSUInteger = 1 << 2;
#[allow(dead_code)]
pub const NSMonthCalendarUnit: NSUInteger = 1 << 3;
#[allow(dead_code)]
pub const NSDayCalendarUnit: NSUInteger = 1 << 4;
#[allow(dead_code)]
pub const NSHourCalendarUnit: NSUInteger = 1 << 5;
#[allow(dead_code)]
pub const NSMinuteCalendarUnit: NSUInteger = 1 << 6;
#[allow(dead_code)]
pub const NSSecondCalendarUnit: NSUInteger = 1 << 7;
#[allow(dead_code)]
pub const NSWeekCalendarUnit: NSUInteger = 1 << 8;
#[allow(dead_code)]
pub const NSWeekdayCalendarUnit: NSUInteger = 1 << 9;
#[allow(dead_code)]
pub const NSWeekdayOrdinalCalendarUnit: NSUInteger = 1 << 10;

#[derive(Default)]
pub struct State {
    current_calendar: Option<id>,
}

struct NSCalendarHostObject {
    identifier: id,
    time_zone: id,
    locale: id,
}
impl HostObject for NSCalendarHostObject {}

struct NSDateComponentsHostObject {
    era: NSInteger,
    year: NSInteger,
    month: NSInteger,
    day: NSInteger,
    hour: NSInteger,
    minute: NSInteger,
    second: NSInteger,
    weekday: NSInteger,
    week: NSInteger,
}
impl HostObject for NSDateComponentsHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// ============================================================================
// NSCalendar
// ============================================================================
@implementation NSCalendar: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSCalendarHostObject {
        identifier: nil,
        time_zone: nil,
        locale: nil,
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

+ (id)currentCalendar {
    if let Some(cal) = env.framework_state.foundation.ns_calendar.current_calendar {
        return cal;
    }
    let cal: id = msg![env; this alloc];
    let cal: id = msg![env; cal init];
    // Retain so the singleton stays alive.
    let _: id = msg![env; cal retain];
    env.framework_state.foundation.ns_calendar.current_calendar = Some(cal);
    cal
}

+ (id)autoupdatingCurrentCalendar {
    msg![env; this currentCalendar]
}

- (id)init {
    this
}

- (id)initWithCalendarIdentifier:(id)identifier {
    env.objc.borrow_mut::<NSCalendarHostObject>(this).identifier = identifier;
    this
}

- (id)calendarIdentifier {
    env.objc.borrow::<NSCalendarHostObject>(this).identifier
}

- (id)timeZone {
    env.objc.borrow::<NSCalendarHostObject>(this).time_zone
}

- (())setTimeZone:(id)tz {
    env.objc.borrow_mut::<NSCalendarHostObject>(this).time_zone = tz;
}

- (id)locale {
    env.objc.borrow::<NSCalendarHostObject>(this).locale
}

- (())setLocale:(id)locale {
    env.objc.borrow_mut::<NSCalendarHostObject>(this).locale = locale;
}

- (id)components:(NSUInteger)_unit_flags fromDate:(id)date {
    // Extract Gregorian components from the date. Ignore the unit_flags
    // mask — we just fill in everything; cost is trivial.
    let ti: NSTimeInterval = msg![env; date timeIntervalSinceReferenceDate];
    let g = CFAbsoluteTimeGetGregorianDate(env, ti, nil);
    let weekday = compute_weekday(g.year, g.month as i32, g.day as i32);
    let comps_class = env.objc.get_known_class("NSDateComponents", &mut env.mem);
    let comps: id = msg![env; comps_class alloc];
    let comps: id = msg![env; comps init];
    let _: () = msg![env; comps setEra:(1 as NSInteger)];
    let _: () = msg![env; comps setYear:(g.year as NSInteger)];
    let _: () = msg![env; comps setMonth:(g.month as NSInteger)];
    let _: () = msg![env; comps setDay:(g.day as NSInteger)];
    let _: () = msg![env; comps setHour:(g.hours as NSInteger)];
    let _: () = msg![env; comps setMinute:(g.minutes as NSInteger)];
    let _: () = msg![env; comps setSecond:(g.seconds as NSInteger)];
    let _: () = msg![env; comps setWeekday:weekday];
    autorelease(env, comps)
}

@end

// ============================================================================
// NSDateComponents
// ============================================================================
@implementation NSDateComponents: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSDateComponentsHostObject {
        era: 1,
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        weekday: 0,
        week: 0,
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

- (id)init { this }

- (NSInteger)era { env.objc.borrow::<NSDateComponentsHostObject>(this).era }
- (())setEra:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).era = v }
- (NSInteger)year { env.objc.borrow::<NSDateComponentsHostObject>(this).year }
- (())setYear:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).year = v }
- (NSInteger)month { env.objc.borrow::<NSDateComponentsHostObject>(this).month }
- (())setMonth:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).month = v }
- (NSInteger)day { env.objc.borrow::<NSDateComponentsHostObject>(this).day }
- (())setDay:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).day = v }
- (NSInteger)hour { env.objc.borrow::<NSDateComponentsHostObject>(this).hour }
- (())setHour:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).hour = v }
- (NSInteger)minute { env.objc.borrow::<NSDateComponentsHostObject>(this).minute }
- (())setMinute:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).minute = v }
- (NSInteger)second { env.objc.borrow::<NSDateComponentsHostObject>(this).second }
- (())setSecond:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).second = v }
- (NSInteger)weekday { env.objc.borrow::<NSDateComponentsHostObject>(this).weekday }
- (())setWeekday:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).weekday = v }
- (NSInteger)week { env.objc.borrow::<NSDateComponentsHostObject>(this).week }
- (())setWeek:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).week = v }

@end

};

/// Compute weekday for a Gregorian date using Zeller's congruence.
/// Returns 1=Sunday, 2=Monday, ..., 7=Saturday (matches Cocoa convention).
fn compute_weekday(year: i32, month: i32, day: i32) -> NSInteger {
    let (y, m) = if month < 3 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let k = y % 100;
    let j = y / 100;
    let h = (day + 13 * (m + 1) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // h: 0=Saturday, 1=Sunday, ..., 6=Friday.
    // Convert to Cocoa: 1=Sunday..7=Saturday.
    (((h + 6) % 7) + 1) as NSInteger
}
