program pcritical
critical(stat=istat)
continue
end critical
end

program pchangeteam
change team(newteam)
continue
end team
continue
change team(newteam)
continue
end team (stat=istat)
l: change team(newteam)
continue
end team (stat=istat) l
k: change team(newteam)
continue
end team k
end
