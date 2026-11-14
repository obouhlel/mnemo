# Pilote PostgreSQL : chargé par réflexion, classes JNDI / JMX absentes d'Android (jamais utilisées)
-keep class org.postgresql.** { *; }
-keep class com.ongres.scram.** { *; }
-dontwarn org.postgresql.**
-dontwarn javax.naming.**
-dontwarn javax.management.**
-dontwarn java.lang.management.**
-dontwarn org.osgi.**
-dontwarn waffle.**
-dontwarn com.sun.jna.**
-dontwarn org.slf4j.**
# Markwon
-dontwarn org.commonmark.**
-dontwarn org.checkerframework.**
